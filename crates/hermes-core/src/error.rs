//! Error types for hermes-core library
//!
//! Uses `thiserror` for domain-specific errors with rich context.

use thiserror::Error;

/// Result type alias for hermes-core operations
pub type Result<T> = std::result::Result<T, Error>;

/// Domain-specific errors for Hermes-RS
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
    #[error("SSE parse error at position {position}: {message}")]
    SseParse { position: usize, message: String },

    #[error("Unexpected SSE event type: {0}")]
    UnexpectedSseEvent(String),

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
        body: String,
        retry_after: Option<std::time::Duration>,
    },

    #[error("Rate limited (retry after {retry_after:?})")]
    RateLimited {
        retry_after: std::time::Duration,
    },

    #[error("Authentication failed: {0}")]
    Authentication(String),

    // ========== Agent Errors ==========
    #[error("Agent error: {0}")]
    Agent(String),

    #[error("Max iterations exceeded: {max}")]
    MaxIterationsExceeded { max: usize },

    #[error("Context length exceeded")]
    ContextLengthExceeded,

    #[error("Invalid state transition: {from} -> {to}")]
    InvalidStateTransition { from: String, to: String },

    // ========== Schema Errors ==========
    #[error("Schema generation error: {0}")]
    SchemaGeneration(String),

    #[error("Invalid schema: {0}")]
    InvalidSchema(String),

    // ========== Configuration Errors ==========
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Missing required configuration: {key}")]
    MissingConfig { key: String },
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
                format!("Invalid arguments for tool '{}': {}", name, details)
            }
            Error::Provider { status, .. } => {
                format!("The AI provider returned an error (HTTP {}). Please try again or modify your approach.", status)
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
        assert!(!err.is_self_healing(), "RateLimited should not be self-healing");
    }

    #[test]
    fn test_authentication_not_transient_not_self_healing() {
        let err = Error::Authentication("invalid key".to_string());
        assert!(!err.is_transient(), "Authentication should not be transient");
        assert!(!err.is_self_healing(), "Authentication should not be self-healing");
    }
}
