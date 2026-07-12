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

/// Domain-specific errors for Operant-RS
#[derive(Error, Debug)]
pub enum Error {
    // ========== Client Errors ==========
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

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
                format!("The AI provider returned an error (HTTP {}). {}", status, msg)
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
    }    #[test]
    fn test_authentication_not_transient_not_self_healing() {
        let err = Error::Authentication("invalid key".to_string());
        assert!(!
            err.is_transient(),
            "Authentication should not be transient"
        );
        assert!(!
            err.is_self_healing(),
            "Authentication should not be self-healing"
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
        assert_eq!(msg, "Error from provider (Console): Upstream request failed (invalid_request_error)");
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
            body: r#"{"error":{"message":"Invalid request","type":"invalid_request_error"}}"#.to_string(),
            retry_after: None,
        };
        let user_msg = err.user_message();
        assert!(user_msg.contains("Invalid request"), "user_message should extract the message field: {}", user_msg);
        assert!(user_msg.contains("400"), "user_message should include HTTP status: {}", user_msg);
    }
}
