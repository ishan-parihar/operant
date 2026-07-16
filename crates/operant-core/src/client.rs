//! OpenAI-compatible client with SSE streaming support
//!
//! A lightweight, custom implementation using reqwest and serde.
//! Supports Server-Sent Events for streaming responses.
//! Supports reasoning_content for extended-thinking models.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures::Stream;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, error, info, instrument, warn};

use crate::config::{runtime_config, ClientSettings};
use crate::error::{Error, Result};
use crate::rate_limiter::{
    exponential_backoff_secs, parse_retry_after_header, RateLimitError, RateLimiter,
};
use crate::schema::ToolSchema;

/// OpenAI API client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Base URL for the OpenAI-compatible API
    pub base_url: String,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Default request timeout
    pub timeout: Duration,
    /// Maximum context length (for truncation warnings)
    pub max_context_length: usize,
    /// Rate limit settings for outbound requests.
    pub rate_limit: crate::config::RateLimitSettings,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::from(&runtime_config().client)
    }
}

impl From<&ClientSettings> for ClientConfig {
    fn from(settings: &ClientSettings) -> Self {
        Self {
            base_url: settings.base_url.clone(),
            api_key: settings.api_key.clone(),
            timeout: Duration::from_secs(settings.timeout_secs),
            max_context_length: settings.max_context_length,
            rate_limit: settings.rate_limit.clone(),
        }
    }
}

/// OpenAI-compatible client for chat completions
#[derive(Debug, Clone)]
pub struct OpenAIClient {
    config: ClientConfig,
    http_client: Client,
    /// Token-bucket rate limiter keyed by model name.
    rate_limiter: RateLimiter,
}

impl OpenAIClient {
    /// Create a new OpenAI client
    pub fn new(config: ClientConfig) -> Self {
        // (iter-139 — fixed ponytail-audit bug A23: was .expect() which
        // panics on reqwest builder failure (e.g. TLS backend missing).
        // Fall back to a default client if the configured builder fails.)
        let http_client = Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Failed to build HTTP client with config, falling back to default");
                Client::new()
            });

        let rate_limiter = RateLimiter::new(
            config.rate_limit.bucket_capacity,
            config.rate_limit.bucket_refill_rate,
        );

        Self {
            config,
            http_client,
            rate_limiter,
        }
    }

    /// Create a client from an existing HTTP client handle.
    pub(crate) fn from_shared_http_client(config: ClientConfig, http_client: Client) -> Self {
        let rate_limiter = RateLimiter::new(
            config.rate_limit.bucket_capacity,
            config.rate_limit.bucket_refill_rate,
        );

        Self {
            config,
            http_client,
            rate_limiter,
        }
    }

    pub(crate) fn config_clone(&self) -> ClientConfig {
        self.config.clone()
    }

    pub(crate) fn http_client_clone(&self) -> Client {
        self.http_client.clone()
    }

    /// Create from environment variables
    pub fn from_env() -> Result<Self> {
        let base = runtime_config();
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.is_empty());

        let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or(base.client.base_url);

        Ok(Self::new(ClientConfig {
            base_url,
            api_key: api_key.or(base.client.api_key),
            timeout: Duration::from_secs(base.client.timeout_secs),
            max_context_length: base.client.max_context_length,
            rate_limit: base.client.rate_limit.clone(),
        }))
    }

    /// Build authorization headers
    fn build_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();

        // Content type
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Authorization
        if let Some(ref api_key) = self.config.api_key {
            let auth_value = format!("Bearer {}", api_key);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth_value)
                    .map_err(|_| Error::Config("Invalid API key format".to_string()))?,
            );
        }

        Ok(headers)
    }

    /// Build the chat completions URL
    fn build_url(&self, endpoint: &str) -> Result<reqwest::Url> {
        let base = self.config.base_url.trim_end_matches('/');
        let url = format!("{}/chat/completions{}", base, endpoint);
        reqwest::Url::parse(&url).map_err(|e| Error::InvalidUrl(e.to_string()))
    }

    /// Send a non-streaming chat completion request with rate-limit handling.
    #[instrument(skip(self, messages, tools), fields(model = % model))]
    pub async fn chat(
        &self,
        model: &str,
        messages: &[Message],
        tools: Option<&[ToolSchema]>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<ChatResponse> {
        let request =
            self.build_chat_request(model, messages, tools, false, max_tokens, temperature)?;

        let url = self.build_url("")?;
        let headers = self.build_headers()?;

        let (status, body) = self
            .execute_with_retry(model, url, headers, request)
            .await?;

        if !status.is_success() {
            error!(status = %status, body = %body, "Chat request failed");
            return Err(classify_http_error(status.as_u16(), &body));
        }

        let response: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| Error::ParseResponse(format!("{}: {}", e, body)))?;

        debug!(usage = ?response.usage, "Chat response received");
        Ok(response)
    }

    /// Send a streaming chat completion request with rate-limit checking.
    #[instrument(skip(self, messages, tools), fields(model = % model))]
    pub async fn chat_streaming(
        &self,
        model: &str,
        messages: &[Message],
        tools: Option<&[ToolSchema]>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<ChatStreamResponse> {
        let request =
            self.build_chat_request(model, messages, tools, true, max_tokens, temperature)?;

        let url = self.build_url("")?;
        let headers = self.build_headers()?;

        // Pre-flight rate-limit check (streaming does not retry mid-stream).
        if let Err(e) = self.rate_limiter.check_rate_limit(model).await {
            let retry_after = match e {
                RateLimitError::TooManyRequests { retry_after_secs } => retry_after_secs,
                _ => 60,
            };
            warn!(
                model = %model,
                retry_after_secs = retry_after,
                "Rate limited, rejecting streaming request"
            );
            return Err(Error::RateLimited {
                retry_after: Duration::from_secs(retry_after),
            });
        }

        let max_retries = self.config.rate_limit.max_retries;
        let base_delay = self.config.rate_limit.base_delay_secs;
        let max_delay = self.config.rate_limit.max_delay_secs;

        for attempt in 1..=max_retries {
            let response = self
                .http_client
                .post(url.clone())
                .headers(headers.clone())
                .json(&request)
                .send()
                .await?;

            let status = response.status();
            if status.is_success() {
                info!("Streaming connection established");
                let stream = response.bytes_stream();
                return Ok(ChatStreamResponse::new(stream));
            }

            let body = response.text().await?;

            if status == 429 {
                self.rate_limiter.drain_bucket(model).await;
            }

            // Retry on 5xx if attempts remain.
            if status.as_u16() >= 500 && attempt < max_retries {
                let delay_s = exponential_backoff_secs(attempt, base_delay, max_delay);
                warn!(
                    model = %model,
                    attempt,
                    status = status.as_u16(),
                    delay_secs = delay_s,
                    "Streaming transient server error, retrying",
                );
                tokio::time::sleep(Duration::from_secs(delay_s)).await;
                continue;
            }

            error!(status = %status, body = %body, "Streaming request failed");
            return Err(classify_http_error(status.as_u16(), &body));
        }

        // (iter-139 — fixed ponytail-audit bug A22: was unreachable!() which
        // is technically reachable when max_retries == 0 and the loop body
        // somehow doesn't return. Replaced with an explicit error.)
        Err(crate::error::Error::Agent(
            "retry loop exhausted without returning a result".to_string(),
        ))
    }

    /// Execute an API POST request with rate-limit checking and automatic retry.
    ///
    /// Retry flow:
    /// 1. Check the proactive token-bucket rate limiter.
    /// 2. Send the HTTP POST request.
    /// 3. On 429: parse `Retry-After`, drain the bucket, wait, retry.
    /// 4. On 5xx / network error: exponential backoff, retry.
    /// 5. After exhausting retries: return the last error.
    async fn execute_with_retry(
        &self,
        model: &str,
        url: reqwest::Url,
        headers: HeaderMap,
        body: serde_json::Value,
    ) -> Result<(reqwest::StatusCode, String)> {
        let max_retries = self.config.rate_limit.max_retries;
        let base_delay = self.config.rate_limit.base_delay_secs;
        let max_delay = self.config.rate_limit.max_delay_secs;

        for attempt in 1..=max_retries {
            // 1. Proactive rate-limit check (consume a token).
            if let Err(e) = self.rate_limiter.check_rate_limit(model).await {
                let wait = match e {
                    RateLimitError::TooManyRequests { retry_after_secs } => {
                        Duration::from_secs(retry_after_secs)
                    }
                    _ => Duration::from_secs(base_delay),
                };
                debug!(
                    model = %model,
                    wait_ms = wait.as_millis(),
                    "Proactive rate limit engaged, waiting",
                );
                if attempt < max_retries {
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(Error::RateLimited { retry_after: wait });
            }

            // 2. Send the HTTP request.
            let response = match self
                .http_client
                .post(url.clone())
                .headers(headers.clone())
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if attempt < max_retries {
                        let delay_s = exponential_backoff_secs(attempt, base_delay, max_delay);
                        warn!(
                            model = %model,
                            attempt,
                            delay_secs = delay_s,
                            error = %e,
                            "Network error, retrying",
                        );
                        tokio::time::sleep(Duration::from_secs(delay_s)).await;
                        continue;
                    }
                    return Err(Error::Network(e));
                }
            };

            let status = response.status();
            let retry_after_hdr = parse_retry_after_header(response.headers());
            let body_text = response.text().await.map_err(Error::Network)?;

            // 3. Handle 429 rate limiting.
            if status == 429 {
                self.rate_limiter.drain_bucket(model).await;

                let retry_after = retry_after_hdr.unwrap_or_else(|| {
                    Duration::from_secs(exponential_backoff_secs(attempt, base_delay, max_delay))
                });

                if attempt < max_retries {
                    warn!(
                        model = %model,
                        attempt,
                        retry_after_ms = retry_after.as_millis(),
                        "HTTP 429, retrying after backoff",
                    );
                    tokio::time::sleep(retry_after).await;
                    continue;
                }

                error!(
                    model = %model,
                    status = %status,
                    body = %body_text,
                    "Request failed after exhausting retries",
                );
                return Err(classify_http_error(status.as_u16(), &body_text));
            }

            // 4. Handle transient server errors.
            if status.as_u16() >= 500 && attempt < max_retries {
                let delay_s = exponential_backoff_secs(attempt, base_delay, max_delay);
                warn!(
                    model = %model,
                    attempt,
                    status = status.as_u16(),
                    delay_secs = delay_s,
                    "Transient server error, retrying",
                );
                tokio::time::sleep(Duration::from_secs(delay_s)).await;
                continue;
            }

            // 5. Success or non-retryable error — return to caller.
            return Ok((status, body_text));
        }

        unreachable!("Retry loop always returns or breaks");
    }

    /// Recursively remove null values and $schema from JSON Schema objects.
    fn clean_schema(obj: &mut serde_json::Map<String, Value>) {
        obj.remove("$schema");
        obj.retain(|_, v| !v.is_null());
        for value in obj.values_mut() {
            if let Some(inner) = value.as_object_mut() {
                Self::clean_schema(inner);
            }
        }
    }

    ///Build the chat request payload
    fn build_chat_request(
        &self,
        model: &str,
        messages: &[Message],
        tools: Option<&[ToolSchema]>,
        stream: bool,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    ) -> Result<serde_json::Value> {
        let mut request = json!({
            "model": model,
            "messages": messages.iter().map(|m| m.to_value()).collect::<Vec<_>>(),
            "stream": stream,
        });

        // Ask OpenAI-compatible providers to include a final usage-only chunk
        // when streaming, so streaming mode can report real token/cost data
        // the same way non-streaming responses already do.
        if stream {
            request["stream_options"] = json!({"include_usage": true});
        }

        // Pass through max_tokens / temperature if the caller set them.
        // Previously these fields were parsed from config into ChatRequest
        // but never sent to the provider — the OpenAI adapter dropped them,
        // so users who set max_tokens=8192 or temperature=0.5 in their
        // config saw no effect.
        if let Some(max_tokens) = max_tokens {
            request["max_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = temperature {
            request["temperature"] = json!(temperature);
        }

        if let Some(tools) = tools {
            if !tools.is_empty() {
                let tools_array: Vec<Value> = tools
                    .iter()
                    .map(|t| {
                        let mut params = t.parameters.clone();
                        if let Some(obj) = params.as_object_mut() {
                            Self::clean_schema(obj);
                        }
                        json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": params
                            }
                        })
                    })
                    .collect();
                request["tools"] = json!(tools_array);
            }
        }

        Ok(request)
    }
}

/// Chat message
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

/// A chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub reasoning: Option<String>,
    pub name: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Provider-specific extra fields (e.g. Google Gemini thought_signature)
    pub extra_content: Option<serde_json::Value>,
}

impl Message {
    /// Create a new message
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        }
    }

    /// Create a system message
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    /// Create a user message
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    /// Create an assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    /// Create a tool message
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            reasoning: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
            tool_calls: None,
            extra_content: None,
        }
    }

    /// Add tool calls to the message
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = Some(tool_calls);
        self
    }

    /// Add reasoning content to the message.
    ///
    /// Stores any non-empty string verbatim — including whitespace-only
    /// values like `" "`. DeepSeek V4 Pro thinking mode and Kimi/Moonshot
    /// thinking mode both pad missing reasoning with a single space when
    /// echoing back tool-call assistant turns; trimming that pad here would
    /// cause the round-tripped message to fail the provider's
    /// `reasoning_content` echo-back validation on the next request.
    /// Refs: operant-agent #15250, #17341 (DeepSeek V4 Pro tightened to
    /// reject empty string; a single space satisfies the validator).
    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        let reasoning = reasoning.into();
        if !reasoning.is_empty() {
            self.reasoning = Some(reasoning);
        }
        self
    }

    /// Add provider-specific extra content (e.g. Google Gemini thought_signature)
    pub fn with_extra_content(mut self, extra: serde_json::Value) -> Self {
        self.extra_content = Some(extra);
        self
    }

    fn repair_json_simple(s: &str) -> String {
        let mut result = s.to_string();
        let mut brace_depth = 0i32;
        let mut bracket_depth = 0i32;
        let mut in_string = false;
        let mut escape_next = false;

        for ch in result.chars() {
            if escape_next {
                escape_next = false;
                continue;
            }
            if ch == '\\' && in_string {
                escape_next = true;
                continue;
            }
            if ch == '"' {
                in_string = !in_string;
                continue;
            }
            if !in_string {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => brace_depth -= 1,
                    '[' => bracket_depth += 1,
                    ']' => bracket_depth -= 1,
                    _ => {}
                }
            }
        }

        if in_string {
            result.push('"');
        }

        let mut trimmed = result.trim_end().to_string();
        while trimmed.ends_with(',') || trimmed.ends_with(':') {
            trimmed.pop();
            trimmed = trimmed.trim_end().to_string();
        }

        for _ in 0..bracket_depth.max(0) {
            trimmed.push(']');
        }
        for _ in 0..brace_depth.max(0) {
            trimmed.push('}');
        }

        trimmed
    }

    /// Convert to JSON value for API
    fn to_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("role".to_string(), json!(self.role.as_str()));

        if let Some(ref tool_calls) = self.tool_calls {
            let tc_array: Vec<Value> = tool_calls
                .iter()
                .map(|tc| {
                    let cleaned_args = {
                        let args = &tc.function.arguments;
                        if serde_json::from_str::<Value>(args).is_ok() {
                            args.clone()
                        } else {
                            let trimmed = args.trim();
                            if trimmed.is_empty() {
                                "{}".to_string()
                            } else {
                                let repaired = Self::repair_json_simple(trimmed);
                                if serde_json::from_str::<Value>(&repaired).is_ok() {
                                    repaired
                                } else {
                                    "{}".to_string()
                                }
                            }
                        }
                    };
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.function.name,
                            "arguments": cleaned_args
                        }
                    })
                })
                .collect();
            map.insert("tool_calls".to_string(), json!(tc_array));
            map.insert("content".to_string(), json!(self.content));
        } else {
            map.insert("content".to_string(), json!(self.content));
        }

        if let Some(ref name) = self.name {
            // Only include name for non-tool messages. Some providers (Google Gemini)
            // reject tool messages with extra fields beyond role/content/tool_call_id.
            if self.role != Role::Tool {
                map.insert("name".to_string(), json!(name));
            }
        }
        if let Some(ref tool_call_id) = self.tool_call_id {
            map.insert("tool_call_id".to_string(), json!(tool_call_id));
        }

        // Include provider-specific extra content (e.g. Google Gemini thought_signature)
        if let Some(ref extra) = self.extra_content {
            if let Some(obj) = extra.as_object() {
                for (k, v) in obj {
                    map.insert(k.clone(), v.clone());
                }
            }
        }

        // DeepSeek V4 Pro / Kimi / MiMo thinking-mode providers reject
        // assistant messages that carry `tool_calls` but no `reasoning_content`
        // on the next replay (HTTP 400 "The reasoning_content in the thinking
        // mode must be passed back to the API"). Empty string is also rejected
        // by V4 Pro — a single space is the minimal value that satisfies the
        // validator without leaking fabricated reasoning. Non-thinking
        // providers ignore the extra field, so this is safe to emit
        // unconditionally for tool-call assistant turns.
        // Refs: operant-agent #15250, #17341, #17400.
        let has_tool_calls = self.tool_calls.is_some();
        match self.reasoning.as_deref() {
            Some(reasoning) if !reasoning.is_empty() => {
                map.insert("reasoning_content".to_string(), json!(reasoning));
            }
            _ if self.role == Role::Assistant && has_tool_calls => {
                map.insert("reasoning_content".to_string(), json!(" "));
            }
            _ => {}
        }

        Value::Object(map)
    }
}

impl Default for Message {
    fn default() -> Self {
        Self {
            role: Role::User,
            content: String::new(),
            reasoning: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
            extra_content: None,
        }
    }
}

/// A tool call from the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub function: ToolCallFunction,
}

/// Function in a tool call
///
/// Both `name` and `arguments` default to empty strings during deserialization.
/// This is required because OpenAI-compatible streaming providers send the
/// function name in the FIRST `tool_calls` delta of an SSE stream and emit
/// subsequent deltas with only an `arguments` fragment (and no `name`). Without
/// these defaults the inner deserializer would error on every continuation
/// delta, making the parent `Option<ToolCallFunction>` fail and dropping all
/// subsequent argument chunks. The agent would then see an empty `arguments`
/// string and produce `Invalid JSON: EOF while parsing a value at line 1
/// column 0` for every tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

/// Chat completion response (non-streaming)
#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

/// A completion choice
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub index: usize,
    pub message: MessageDelta,
    pub finish_reason: Option<String>,
}

/// Message delta from API (non-streaming)
#[derive(Debug, Clone, Deserialize)]
pub struct MessageDelta {
    pub role: Option<Role>,
    pub content: Option<String>,
    /// Reasoning content from extended-thinking models (e.g. DeepSeek, OpenAI o1)
    #[serde(
        default,
        alias = "reasoning_content",
        alias = "reasoning",
        alias = "reasoning_context"
    )]
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// Tool call delta
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCallDelta {
    #[serde(default)]
    pub index: usize,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<ToolCallFunction>,
}

/// API usage statistics
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// SSE streaming event from the OpenAI API
#[derive(Debug, Clone, Deserialize)]
pub struct ChatStreamEvent {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    /// Present only on the final usage-only chunk when the request set
    /// `stream_options.include_usage`.
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// A streaming choice
#[derive(Debug, Clone, Deserialize)]
pub struct StreamChoice {
    #[serde(default)]
    pub index: usize,
    pub delta: StreamingMessageDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Message delta from streaming API
#[derive(Debug, Clone, Deserialize)]
pub struct StreamingMessageDelta {
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub content: Option<String>,
    /// Reasoning content from extended-thinking models (e.g. DeepSeek, OpenAI o1)
    #[serde(
        default,
        alias = "reasoning_content",
        alias = "reasoning",
        alias = "reasoning_context"
    )]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamingToolCallDelta>>,
    /// Provider-specific extra content (e.g. Google Gemini thought_signature)
    #[serde(default, alias = "extra_content")]
    pub extra_content: Option<serde_json::Value>,
}

/// Tool call delta for streaming
#[derive(Debug, Clone, Deserialize)]
pub struct StreamingToolCallDelta {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub function: Option<ToolCallFunction>,
}

/// Classify an HTTP error from the provider API into a structured Error variant.
///
/// Uses the HTTP status code and response body to produce the most specific
/// error variant, extracting retry timing from the body when available.
///
/// The body is sanitized at construction time (newlines/carriage returns
/// stripped, truncated to 500 chars) so the Display output is readable.
pub(crate) fn classify_http_error(status: u16, body: &str) -> Error {
    let retry_after = parse_retry_after_from_body(body);
    let clean_body = sanitize_error_body(body);
    match status {
        401 | 403 => Error::Authentication(body.to_string()),
        429 => Error::RateLimited {
            retry_after: retry_after.unwrap_or(Duration::from_secs(5)),
        },
        s if s >= 500 => Error::Provider {
            status,
            body: clean_body,
            retry_after,
        },
        _ => Error::Provider {
            status,
            body: clean_body,
            retry_after: None,
        },
    }
}

/// Sanitize a provider error body for display — strip newlines, carriage
/// returns, collapse whitespace, truncate to 500 chars. Without this,
/// raw provider JSON responses with `\r\r\r...` padding produce unreadable
/// error messages in the TUI/CLI.
fn sanitize_error_body(body: &str) -> String {
    // Strip CR/LF and collapse whitespace
    let clean = body
        .replace("\r\n", " ")
        .replace('\r', " ")
        .replace('\n', " ");
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.len() > 500 {
        format!("{}...", &clean[..500])
    } else {
        clean
    }
}

/// Try to extract a `retry_after` duration from the provider's JSON error body.
///
/// Checks for `retry_after` as a number (seconds) or a parseable string.
fn parse_retry_after_from_body(body: &str) -> Option<Duration> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    if let Some(seconds) = json.get("retry_after").and_then(|v| v.as_f64()) {
        return Some(Duration::from_secs_f64(seconds));
    }
    if let Some(seconds_str) = json.get("retry_after").and_then(|v| v.as_str()) {
        if let Ok(seconds) = seconds_str.parse::<f64>() {
            return Some(Duration::from_secs_f64(seconds));
        }
    }
    None
}

/// SSE streaming response wrapper
pub struct ChatStreamResponse {
    inner: Box<dyn Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin>,
    buffer: String,
}

impl ChatStreamResponse {
    pub fn new(
        stream: impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    ) -> Self {
        Self {
            inner: Box::new(stream),
            buffer: String::new(),
        }
    }
}

impl Stream for ChatStreamResponse {
    type Item = crate::error::Result<ChatStreamEvent>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(event) = try_parse_next_sse_event(&mut this.buffer, false) {
                return Poll::Ready(Some(Ok(event)));
            }

            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                        this.buffer.push_str(&text);
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(Error::Network(e)))),
                Poll::Ready(None) => {
                    return Poll::Ready(try_parse_next_sse_event(&mut this.buffer, true).map(Ok));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

fn try_parse_next_sse_event(buffer: &mut String, allow_partial: bool) -> Option<ChatStreamEvent> {
    normalize_sse_buffer(buffer);

    let event_end = if let Some(index) = buffer.find("\n\n") {
        index
    } else if allow_partial && !buffer.trim().is_empty() {
        buffer.len()
    } else {
        return None;
    };

    let event_data = buffer[..event_end].to_string();
    let drain_len = if event_end < buffer.len() {
        event_end + 2
    } else {
        event_end
    };
    buffer.drain(..drain_len);

    let payload = event_data
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");

    if payload.is_empty() {
        return None;
    }

    if payload.trim() == "[DONE]" {
        return None;
    }

    match serde_json::from_str::<ChatStreamEvent>(payload.trim()) {
        Ok(event) => Some(event),
        Err(e) => {
            if let Some(json_start) = payload.find('{') {
                let potential_json = &payload[json_start..];
                if let Ok(event) = serde_json::from_str::<ChatStreamEvent>(potential_json.trim()) {
                    return Some(event);
                }
            }
            debug!(error = %e, payload = %payload, "Failed to parse SSE event");
            None
        }
    }
}

fn normalize_sse_buffer(buffer: &mut String) {
    if buffer.contains('\r') {
        *buffer = buffer.replace("\r\n", "\n").replace('\r', "\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_to_value() {
        let msg = Message::user("Hello, world!");
        let value = msg.to_value();

        assert_eq!(value["role"], "user");
        assert_eq!(value["content"], "Hello, world!");
    }

    #[test]
    fn test_tool_message() {
        let msg = Message::tool("call_123", "Result: 42");
        let value = msg.to_value();

        assert_eq!(value["role"], "tool");
        assert_eq!(value["tool_call_id"], "call_123");
    }

    #[test]
    fn test_assistant_message_with_reasoning_serializes_reasoning_content() {
        let msg = Message::assistant("Hello").with_reasoning("deep thought");
        let value = msg.to_value();

        assert_eq!(value["role"], "assistant");
        assert_eq!(value["content"], "Hello");
        assert_eq!(value["reasoning_content"], "deep thought");
    }

    #[test]
    fn test_assistant_message_with_tool_calls_and_reasoning() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            function: ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{}".to_string(),
            },
        }];
        let msg = Message::assistant("")
            .with_tool_calls(tool_calls)
            .with_reasoning("need to check time");
        let value = msg.to_value();

        assert_eq!(value["role"], "assistant");
        assert_eq!(value["reasoning_content"], "need to check time");
        assert!(value.get("tool_calls").is_some(), "should have tool_calls");
    }

    #[test]
    fn assistant_tool_calls_without_reasoning_pads_reasoning_content() {
        // Regression: DeepSeek V4 Pro thinking mode (and Kimi / MiMo) returns
        // HTTP 400 "The reasoning_content in the thinking mode must be passed
        // back to the API" when an assistant tool-call message is replayed
        // without `reasoning_content`. We pad with a single space — empty
        // string is rejected, " " is the minimum that satisfies the validator.
        let tool_calls = vec![ToolCall {
            id: "call_pad".to_string(),
            function: ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{}".to_string(),
            },
        }];
        let msg = Message::assistant("").with_tool_calls(tool_calls);

        let value = msg.to_value();
        assert_eq!(
            value["reasoning_content"], " ",
            "missing reasoning on tool-call assistant turn must be padded to ' '"
        );
    }

    #[test]
    fn assistant_without_tool_calls_does_not_pad_reasoning_content() {
        // Plain assistant text turns without tool_calls don't trigger the
        // DeepSeek thinking-mode echo-back rule, so we must NOT inject the
        // pad — that would corrupt sessions for non-thinking providers.
        let msg = Message::assistant("hello");
        let value = msg.to_value();
        assert!(
            value.get("reasoning_content").is_none(),
            "non-tool-call assistant turns must not synthesize reasoning_content"
        );
    }

    #[test]
    fn with_reasoning_preserves_whitespace_pad() {
        // DeepSeek echoes back its own " " pad on subsequent replays. We must
        // preserve that verbatim so the next replay round-trips successfully.
        let msg = Message::assistant("").with_reasoning(" ");
        assert_eq!(msg.reasoning.as_deref(), Some(" "));
    }

    #[test]
    fn with_reasoning_drops_empty_string() {
        let msg = Message::assistant("").with_reasoning("");
        assert!(msg.reasoning.is_none());
    }

    #[test]
    fn user_and_tool_messages_never_get_reasoning_pad() {
        // The pad applies only to assistant tool-call turns. User and tool
        // messages must remain untouched even when tool_calls are absent.
        let user = Message::user("hi").to_value();
        assert!(user.get("reasoning_content").is_none());

        let tool = Message::tool("call_x", "result").to_value();
        assert!(tool.get("reasoning_content").is_none());
    }

    #[test]
    fn test_reasoning_context_alias_deserializes() {
        let value = serde_json::json!({
            "role": "assistant",
            "reasoning_context": "<think>checking</think>"
        });

        let delta: StreamingMessageDelta =
            serde_json::from_value(value).expect("streaming delta should deserialize");

        assert_eq!(
            delta.reasoning_content.as_deref(),
            Some("<think>checking</think>")
        );
    }

    #[test]
    fn streaming_parser_handles_crlf_events() {
        let mut buffer = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"demo\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\r\n\r\n".to_string();
        let event = try_parse_next_sse_event(&mut buffer, false).expect("event should parse");

        assert_eq!(event.choices.len(), 1);
        assert_eq!(event.choices[0].delta.content.as_deref(), Some("Hello"));
        assert!(buffer.is_empty());
    }

    #[test]
    fn streaming_parser_handles_partial_final_event() {
        let mut buffer = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"demo\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Done\"},\"finish_reason\":\"stop\"}]}".to_string();
        let event =
            try_parse_next_sse_event(&mut buffer, true).expect("trailing event should parse");

        assert_eq!(event.choices[0].delta.content.as_deref(), Some("Done"));
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn test_client_from_env() {
        // This will succeed even without env vars (uses defaults)
        let client = OpenAIClient::from_env();
        assert!(client.is_ok());
    }

    // --- classify_http_error tests ---

    #[test]
    fn classify_401_as_authentication() {
        let err = classify_http_error(401, "Invalid API key");
        assert!(matches!(err, Error::Authentication(_)));
    }

    #[test]
    fn classify_403_as_authentication() {
        let err = classify_http_error(403, "Forbidden");
        assert!(matches!(err, Error::Authentication(_)));
    }

    #[test]
    fn classify_429_as_rate_limited() {
        let err = classify_http_error(429, "Too Many Requests");
        assert!(matches!(err, Error::RateLimited { .. }));
    }

    #[test]
    fn classify_429_parses_retry_after() {
        let body = r#"{"error": "rate limit", "retry_after": 12.5}"#;
        let err = classify_http_error(429, body);
        match err {
            Error::RateLimited { retry_after } => {
                assert_eq!(retry_after.as_secs_f64(), 12.5);
            }
            _ => panic!("expected RateLimited, got {:?}", err),
        }
    }

    #[test]
    fn classify_500_as_provider() {
        let err = classify_http_error(500, "Internal error");
        match err {
            Error::Provider {
                status,
                ref body,
                retry_after: _,
            } => {
                assert_eq!(status, 500);
                assert_eq!(body, "Internal error");
            }
            _ => panic!("expected Provider, got {:?}", err),
        }
    }

    #[test]
    fn classify_502_as_transient_provider() {
        let err = classify_http_error(502, "Bad Gateway");
        assert!(err.is_transient());
    }

    #[test]
    fn classify_400_as_non_transient_provider() {
        let err = classify_http_error(400, "Bad Request");
        assert!(matches!(err, Error::Provider { status: 400, .. }));
        assert!(!err.is_transient());
    }

    #[test]
    fn classify_http_error_parses_retry_after_string() {
        let body = r#"{"retry_after": "30"}"#;
        let err = classify_http_error(429, body);
        match err {
            Error::RateLimited { retry_after } => {
                assert_eq!(retry_after.as_secs(), 30);
            }
            _ => panic!("expected RateLimited, got {:?}", err),
        }
    }

    #[test]
    fn classify_http_error_invalid_body_does_not_panic() {
        // Invalid JSON should not crash; falls through to default classification
        let err = classify_http_error(429, "not json at all");
        assert!(matches!(err, Error::RateLimited { .. }));
    }

    // --- iter-23: max_tokens / temperature pass-through tests ---

    /// `build_chat_request` now threads `max_tokens` and `temperature` into
    /// the JSON body. Previously these fields existed on `ChatRequest` but
    /// were dropped by the OpenAI adapter — config-level values never reached
    /// the provider.
    #[test]
    fn build_chat_request_includes_max_tokens_and_temperature_when_set() {
        let client = OpenAIClient::new(ClientConfig::default());
        let request = client
            .build_chat_request(
                "gpt-4o",
                &[Message::user("hi")],
                None,
                false,
                Some(8192),
                Some(0.5),
            )
            .expect("request should build");

        assert_eq!(request["max_tokens"], 8192);
        assert_eq!(request["temperature"], 0.5);
    }

    #[test]
    fn build_chat_request_omits_max_tokens_and_temperature_when_none() {
        let client = OpenAIClient::new(ClientConfig::default());
        let request = client
            .build_chat_request("gpt-4o", &[Message::user("hi")], None, false, None, None)
            .expect("request should build");

        // Both fields should be absent (not null) so we don't send default
        // values that might conflict with provider-side defaults.
        assert!(request.get("max_tokens").is_none());
        assert!(request.get("temperature").is_none());
    }

    #[test]
    fn build_chat_request_includes_only_max_tokens_when_temperature_none() {
        let client = OpenAIClient::new(ClientConfig::default());
        let request = client
            .build_chat_request(
                "gpt-4o",
                &[Message::user("hi")],
                None,
                false,
                Some(4096),
                None,
            )
            .expect("request should build");

        assert_eq!(request["max_tokens"], 4096);
        assert!(request.get("temperature").is_none());
    }
}
