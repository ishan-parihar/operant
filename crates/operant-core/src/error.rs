//! Error types for operant-core library
//!
//! Uses `thiserror` for domain-specific errors with rich context.

use thiserror::Error;

/// Result type alias for operant-core operations
pub type Result<T> = std::result::Result<T, Error>;

/// Try to extract a human-readable message from a provider error body.
///
/// Provider error bodies are typically JSON like:
///   {"error":{"message":"Internal server error"}}
///   {"type":"error","error":{"type":"error","message":"..."}}
///   {"error":{"message":"Error from provider (Console): ...","type":"invalid_request_error"}}
///
/// Falls back to the raw body if JSON parsing fails or no message field is found.
fn extract_provider_message(body: &str) -> String {
    // Quick check: if the body doesn't look like JSON, return as-is.
    let trimmed = body.trim();
    if !trimmed.starts_with('{') {
        return body.to_string();
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Try common JSON structures:
        // 1. {"error":{"message":"..."}}
        // 2. {"type":"error","error":{"message":"..."}}
        // 3. {"message":"..."}
        if let Some(msg) = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            // Also try to extract the error type for context
            let error_type = val
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            if error_type.is_empty() || error_type == "error" {
                return msg.to_string();
            }
            return format!("{} ({})", msg, error_type);
        }
        // 3. {"message":"..."}
        if let Some(msg) = val.get("message").and_then(|m| m.as_str()) {
            return msg.to_string();
        }
    }

    // Fallback: strip newlines and truncate.
    let clean = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.len() > 200 {
        format!("{}...", &clean[..200])
    } else {
        clean
    }
}

/// Wrapper around `reqwest::Error` whose `Display`/`Debug` scrub secrets.
///
/// `reqwest` error strings embed the full request URL — and provider URLs
/// routinely contain API keys / bot tokens (e.g. Telegram's
/// `https://api.telegram.org/bot<TOKEN>/sendMessage`, query-param keys).
/// Without this wrapper, any `?` on a failed `.send()` would leak the
/// credential into every log line, `channel send` error, and gateway status.
/// Hermes `_redact_telegram_error_text` / `_redact_discord_error_text`
/// parity — scrub at the error boundary instead of at every call site.
pub struct RedactedReqwestError(pub reqwest::Error);

impl std::fmt::Display for RedactedReqwestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            crate::redaction::redact_sensitive_text_if_enabled(&self.0.to_string())
        )
    }
}

impl std::fmt::Debug for RedactedReqwestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for RedactedReqwestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<reqwest::Error> for RedactedReqwestError {
    fn from(e: reqwest::Error) -> Self {
        Self(e)
    }
}

/// `?` on a `reqwest::Result` converts through the redacting wrapper, so
/// the URL-embedded token never survives into the `Network` error text.
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Network(RedactedReqwestError(e))
    }
}

/// Domain-specific errors for Operant-RS
#[derive(Error, Debug)]
pub enum Error {
    // ========== Client Errors ==========
    #[error("Network error: {0}")]
    Network(#[from] RedactedReqwestError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse response: {0}")]
    ParseResponse(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Missing API key")]
    MissingApiKey,

    // ========== Streaming Errors ==========
    #[error("Incomplete SSE message")]
    IncompleteSseMessage,

    /// The provider's response stream died mid-body and the turn's retry
    /// budget ran out before a re-issue succeeded.
    ///
    /// Distinct from an empty-but-successful response on purpose. An empty
    /// response means the provider answered and had nothing to say — the
    /// nudge ladder retries it and the user is told the provider was
    /// overloaded. A dropped stream means the provider *failed to deliver*:
    /// no retry recovered inside the wall-clock budget, and the content the
    /// user asked for may have been computed upstream and lost. Those are
    /// different facts about the user's request, and reporting one as the
    /// other sends people to debug the wrong layer.
    ///
    /// Carries the underlying error text so the give-up is actionable rather
    /// than a bare "failed": the operator needs to know it was a transport
    /// reset, not a content filter or a bad key.
    ///
    /// The field is `cause`, not `source`: `thiserror` reads a field named
    /// `source` as the chained `std::error::Error`, and a `String` is not
    /// one. The text is captured deliberately — the caller has already
    /// rendered it — so there is no error value left to chain.
    #[error("The model provider's stream died mid-response and did not recover: {cause}")]
    StreamDied {
        /// The underlying transport error, verbatim.
        cause: String,
        /// How many times the request was re-issued before giving up.
        attempts: usize,
        /// Wall-clock seconds spent on the dead stream before giving up.
        elapsed_secs: u64,
    },

    // ========== Tool Errors ==========
    #[error("Tool not found: {name}")]
    ToolNotFound { name: String },

    #[error("Tool execution failed: {name} - {source}")]
    ToolExecution {
        name: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Tool timeout: {name} (exceeded {timeout:?})")]
    ToolTimeout {
        name: String,
        timeout: std::time::Duration,
    },

    #[error("Invalid tool arguments for {name}: {details}")]
    InvalidToolArgs { name: String, details: String },

    #[error("Tool cancelled: {name}")]
    ToolCancelled { name: String },

    // ========== Parser Errors ==========
    #[error("XML parse error: {0}")]
    XmlParse(String),

    #[error("Incomplete XML: {context}")]
    IncompleteXml { context: String },

    #[error("JSON decode error: {0}")]
    JsonDecode(#[from] serde_json::Error),

    // ========== Provider Errors ==========
    #[error("Provider API error: HTTP {status} - {body}")]
    Provider {
        status: u16,
        /// Raw body from the provider API response.
        body: String,
        retry_after: Option<std::time::Duration>,
    },

    #[error("Rate limited (retry after {retry_after:?})")]
    RateLimited { retry_after: std::time::Duration },

    #[error("Authentication failed: {0}")]
    Authentication(String),

    // ========== Agent Errors ==========
    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Max iterations exceeded: {max}")]
    MaxIterationsExceeded { max: usize },

    #[error("Context length exceeded")]
    ContextLengthExceeded,

    // ========== Configuration Errors ==========
    #[error("Configuration error: {0}")]
    Config(String),
}

impl Error {
    /// Returns whether this error indicates a transient failure that might succeed on retry
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Network(_)
            | Error::IncompleteSseMessage
            | Error::ToolTimeout { .. }
            | Error::IncompleteXml { .. }
            | Error::RateLimited { .. } => true,
            Error::Provider { status, .. } if *status >= 500 => true,
            _ => false,
        }
    }

    /// Returns whether this error should trigger self-healing (re-prompt the LLM)
    pub fn is_self_healing(&self) -> bool {
        matches!(
            self,
            Error::ToolNotFound { .. }
                | Error::InvalidToolArgs { .. }
                | Error::ToolExecution { .. }
                | Error::XmlParse(_)
                | Error::Provider { .. }
                | Error::Agent(_)
        )
    }

    /// Get a user-friendly error message for display
    pub fn user_message(&self) -> String {
        match self {
            Error::ToolNotFound { name } => {
                format!("The requested tool '{}' is not available.", name)
            }
            Error::ToolExecution { name, .. } => {
                format!("Tool '{}' encountered an error during execution.", name)
            }
            Error::ToolTimeout { name, .. } => {
                format!("Tool '{}' timed out.", name)
            }
            Error::InvalidToolArgs { name, details } => {
                format!("Tool '{}' received invalid arguments: {}", name, details)
            }
            Error::Provider { status, body, .. } => {
                let msg = extract_provider_message(body);
                format!(
                    "The AI provider returned an error (HTTP {}). {}",
                    status, msg
                )
            }
            Error::RateLimited { .. } => {
                "Rate limit exceeded. Waiting before retrying.".to_string()
            }
            Error::Authentication(_) => {
                "Authentication with the AI provider failed. Please check your API key.".to_string()
            }
            Error::MaxIterationsExceeded { max } => {
                format!("Maximum iterations ({}) exceeded.", max)
            }
            Error::StreamDied {
                cause,
                attempts,
                elapsed_secs,
            } => {
                format!(
                    "The model provider's stream died mid-response and did not recover \
                     after {attempts} retr{} over {elapsed_secs}s ({cause}). \
                     Your message was not lost — reply 'continue' to retry it.",
                    if *attempts == 1 { "y" } else { "ies" }
                )
            }
            Error::ContextLengthExceeded => {
                "The conversation has exceeded the maximum context length.".to_string()
            }
            _ => self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification() {
        let tool_not_found = Error::ToolNotFound {
            name: "test_tool".to_string(),
        };
        assert!(tool_not_found.is_self_healing());
        assert!(!tool_not_found.is_transient());
    }

    #[test]
    fn test_provider_500_is_transient_and_self_healing() {
        let err = Error::Provider {
            status: 500,
            body: "Internal Server Error".to_string(),
            retry_after: None,
        };
        assert!(err.is_transient(), "Provider 500 should be transient");
        assert!(err.is_self_healing(), "Provider 500 should be self-healing");
    }

    #[test]
    fn test_provider_400_is_not_transient_but_self_healing() {
        let err = Error::Provider {
            status: 400,
            body: "Bad Request".to_string(),
            retry_after: None,
        };
        assert!(!err.is_transient(), "Provider 400 should not be transient");
        assert!(err.is_self_healing(), "Provider 400 should be self-healing");
    }

    #[test]
    fn test_rate_limited_is_transient_not_self_healing() {
        let err = Error::RateLimited {
            retry_after: std::time::Duration::from_secs(5),
        };
        assert!(err.is_transient(), "RateLimited should be transient");
        assert!(
            !err.is_self_healing(),
            "RateLimited should not be self-healing"
        );
    }
    #[test]
    fn test_authentication_not_transient_not_self_healing() {
        let err = Error::Authentication("invalid key".to_string());
        assert!(
            !err.is_transient(),
            "Authentication should not be transient"
        );
        assert!(
            !err.is_self_healing(),
            "Authentication should not be self-healing"
        );
    }

    /// A dead stream is a *decision*, not a transient condition. If this ever
    /// reports transient, whatever consumes it may start the retry ladder
    /// again — which is precisely the unbounded wait this variant exists to
    /// end. The provider may well be healthy on the next turn; the point is
    /// that this turn has already spent its budget.
    #[test]
    fn stream_died_is_not_transient_and_not_self_healing() {
        let err = Error::StreamDied {
            cause: "Network error: error decoding response body".to_string(),
            attempts: 2,
            elapsed_secs: 120,
        };
        assert!(
            !err.is_transient(),
            "a spent stream-retry budget must not invite another retry ladder"
        );
        assert!(
            !err.is_self_healing(),
            "re-prompting the LLM cannot fix a dead transport"
        );
    }

    /// The message must name the actual failure and the actual cost, and must
    /// tell the user their message survived. Silence after four minutes is
    /// the defect; "something happened" is not a fix.
    #[test]
    fn stream_died_user_message_is_actionable() {
        let msg = Error::StreamDied {
            cause: "Network error: error decoding response body".to_string(),
            attempts: 2,
            elapsed_secs: 120,
        }
        .user_message();
        assert!(msg.contains("stream died"), "got: {msg}");
        assert!(
            msg.contains("error decoding response body"),
            "must carry the underlying cause, got: {msg}"
        );
        assert!(msg.contains("120s"), "must state the cost, got: {msg}");
        assert!(msg.contains("not lost"), "got: {msg}");
        assert!(msg.contains("continue"), "got: {msg}");
    }

    #[test]
    fn stream_died_user_message_agrees_in_singular_and_plural() {
        let one = Error::StreamDied {
            cause: "reset".to_string(),
            attempts: 1,
            elapsed_secs: 45,
        }
        .user_message();
        assert!(one.contains("1 retry"), "got: {one}");
        assert!(!one.contains("1 retries"), "got: {one}");

        let many = Error::StreamDied {
            cause: "reset".to_string(),
            attempts: 3,
            elapsed_secs: 200,
        }
        .user_message();
        assert!(many.contains("3 retries"), "got: {many}");
    }

    /// The distinction the task turns on: a dropped stream and an
    /// empty-but-successful response are different facts and must never
    /// produce the same user-facing text. Empty means the provider answered
    /// with nothing (overload); a dropped stream means it failed to deliver.
    #[test]
    fn stream_died_is_not_conflated_with_empty_response() {
        let died = Error::StreamDied {
            cause: "Network error: error decoding response body".to_string(),
            attempts: 2,
            elapsed_secs: 90,
        }
        .user_message();

        // The empty-response wording the gateway already ships for a
        // successful-but-blank reply (see the CLI's empty-content branch).
        let empty_wording = "returned an empty response after retries";

        assert!(
            !died.contains(empty_wording),
            "a dead stream must not be reported as an empty response: {died}"
        );
        assert!(
            !died.to_lowercase().contains("provider overload"),
            "overload is an empty-response diagnosis, not a transport one: {died}"
        );
    }

    #[test]
    fn test_extract_provider_message_nested_error() {
        // Anthropic-style: {"error":{"message":"...","type":"error"}}
        let body = r#"{"type":"error","error":{"type":"error","message":"Internal server error"}}"#;
        let msg = super::extract_provider_message(body);
        assert_eq!(msg, "Internal server error");
    }

    #[test]
    fn test_extract_provider_message_with_type() {
        // OpenAI-style: {"error":{"message":"...","type":"invalid_request_error"}}
        let body = r#"{"error":{"message":"Error from provider (Console): Upstream request failed","type":"invalid_request_error","param":null,"code":"invalid_request_error"}}"#;
        let msg = super::extract_provider_message(body);
        assert_eq!(
            msg,
            "Error from provider (Console): Upstream request failed (invalid_request_error)"
        );
    }

    #[test]
    fn test_extract_provider_message_simple() {
        let body = r#"{"message":"Bad Request"}"#;
        let msg = super::extract_provider_message(body);
        assert_eq!(msg, "Bad Request");
    }

    #[test]
    fn test_extract_provider_message_non_json() {
        let body = "Internal Server Error";
        let msg = super::extract_provider_message(body);
        assert_eq!(msg, "Internal Server Error");
    }

    #[test]
    fn test_extract_provider_message_fallback() {
        // Malformed JSON that can't be parsed
        let body = "{not valid json";
        let msg = super::extract_provider_message(body);
        assert!(msg.contains("not valid json"));
    }

    #[test]
    fn test_provider_user_message_extracted() {
        let err = Error::Provider {
            status: 400,
            body: r#"{"error":{"message":"Invalid request","type":"invalid_request_error"}}"#
                .to_string(),
            retry_after: None,
        };
        let user_msg = err.user_message();
        assert!(
            user_msg.contains("Invalid request"),
            "user_message should extract the message field: {}",
            user_msg
        );
        assert!(
            user_msg.contains("400"),
            "user_message should include HTTP status: {}",
            user_msg
        );
    }

    /// Telegram bot tokens embedded in reqwest URLs must never survive into
    /// `Network` error Display (hermes `_redact_telegram_error_text` parity).
    ///
    /// reqwest's error constructor is crate-private, so we drive a real
    /// failing request against a dead port to get a genuine `reqwest::Error`
    /// whose text embeds the request URL (including any token in it).
    #[tokio::test]
    async fn network_error_redacts_token_url() {
        let token = "1234567890:AbCdEfGhIjKlMnOpQrStUvWxYzAbCdEfGhX"; // 35-char suffix per TG format
        let client = reqwest::Client::new();
        let url = format!(
            "http://127.0.0.1:1/bot{token}/sendMessage?api_key=sk-abcdef1234567890abcdef1234567890"
        );
        let reqwest_err = client
            .get(&url)
            .send()
            .await
            .expect_err("dead port must fail");
        let wrapped: RedactedReqwestError = reqwest_err.into();
        let display = wrapped.to_string();
        assert!(
            !display.contains(token),
            "bot token leaked into network error: {display}"
        );
        assert!(
            !display.contains("sk-abcdef1234567890abcdef1234567890"),
            "query-param key leaked into network error: {display}"
        );
    }

    /// The `?` conversion path (reqwest::Error -> Error::Network) must also
    /// scrub the token-bearing URL.
    #[tokio::test]
    async fn network_error_conversion_redacts() {
        let client = reqwest::Client::new();
        let url = "http://127.0.0.1:1/v1?api_key=sk-abcdef1234567890abcdef1234567890&x=1";
        let reqwest_err = client
            .get(url)
            .send()
            .await
            .expect_err("dead port must fail");
        let err: Error = reqwest_err.into();
        let display = err.to_string();
        assert!(
            !display.contains("sk-abcdef1234567890abcdef1234567890"),
            "query-param key leaked into converted network error: {display}"
        );
        assert!(display.contains("Network error"), "got: {display}");
    }
}
