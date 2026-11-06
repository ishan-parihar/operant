use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
}

/// Description of a tool for the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Core tool trait — implement for any capability
// `async_trait` puts a bare `#[must_use]` on every async method it
// generates, and these methods return a `Result` — which is ALREADY
// `#[must_use]`. The lint's own suggestion (give the attribute a message)
// cannot be applied to a macro-generated attribute, so the duplication is
// expected here.
#[allow(
    clippy::double_must_use,
    reason = "async_trait's generated #[must_use] on a Result-returning method"
)]
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name (used in LLM function calling)
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// JSON schema for parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with given arguments
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;

    /// Emoji representing this tool (used in channel notifications)
    fn emoji(&self) -> &str {
        "\u{1F527}"
    }

    /// Get the full spec for LLM registration
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}
