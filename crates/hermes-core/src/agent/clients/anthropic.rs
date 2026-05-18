//! Native Anthropic Messages API adapter.
//!
//! Activated by the `anthropic` feature flag.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

use super::super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::{
    ChatResponse, Choice, Message, MessageDelta, Role, ToolCall, ToolCallFunction, Usage,
};
use crate::error::{Error, Result};

/// Anthropic Messages API client.
#[derive(Clone)]
pub struct AnthropicModelClient {
    api_key: String,
    base_url: String,
    http: Client,
}

impl AnthropicModelClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com".to_string(),
            http: Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Convert internal messages to Anthropic format, extracting system prompt.
    fn convert_request(&self, request: &ChatRequest) -> Value {
        let mut system: Option<String> = None;
        let mut messages: Vec<Value> = Vec::new();

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    system = Some(msg.content.clone());
                }
                Role::User => {
                    messages.push(json!({"role": "user", "content": msg.content}));
                }
                Role::Assistant => {
                    let content = self.build_assistant_content(msg);
                    messages.push(json!({"role": "assistant", "content": content}));
                }
                Role::Tool => {
                    messages.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                            "content": msg.content
                        }]
                    }));
                }
            }
        }

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });

        if let Some(sys) = system {
            body["system"] = json!(sys);
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools.iter().map(|t| json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters
            })).collect::<Vec<_>>());
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
                + usage_obj["output_tokens"].as_u64().unwrap_or(0)) as u32,
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

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let body = self.convert_request(&request);
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Agent(format!("Anthropic API error {status}: {text}")));
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

        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            return Err(Error::Agent(format!("Anthropic API error {status}: {text}")));
        }

        let byte_stream = resp.bytes_stream();

        let stream = byte_stream
            .scan(String::new(), |buffer, chunk_result| {
                let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        // Process complete SSE lines
                        while let Some(pos) = buffer.find("\n\n") {
                            let event_block = buffer[..pos].to_string();
                            *buffer = buffer[pos + 2..].to_string();

                            if let Some(chunk) = parse_sse_event(&event_block) {
                                chunks.push(Ok(chunk));
                            }
                        }
                    }
                    Err(e) => chunks.push(Err(Error::Network(e))),
                }
                futures::future::ready(Some(futures::stream::iter(chunks)))
            })
            .flatten();

        Ok(Box::pin(stream))
    }
}

/// Parse a single SSE event block into a StreamChunk.
fn parse_sse_event(block: &str) -> Option<StreamChunk> {
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
                    // Partial JSON for tool input — emit as tool_call argument fragment.
                    let partial = delta["partial_json"].as_str().unwrap_or("");
                    // We need the block index to correlate; use json["index"]
                    let _index = json["index"].as_u64().unwrap_or(0);
                    Some(StreamChunk::new(
                        None,
                        None,
                        Some(vec![ToolCall {
                            id: String::new(), // filled by content_block_start
                            function: ToolCallFunction {
                                name: String::new(),
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
            match block["type"].as_str()? {
                "tool_use" => Some(StreamChunk::new(
                    None,
                    None,
                    Some(vec![ToolCall {
                        id: block["id"].as_str().unwrap_or("").to_string(),
                        function: ToolCallFunction {
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            arguments: String::new(),
                        },
                    }]),
                )),
                _ => None,
            }
        }
        _ => None,
    }
}
