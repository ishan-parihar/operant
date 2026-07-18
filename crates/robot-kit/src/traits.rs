use async_trait::async_trait;
use serde_json::Value;

/// Result of a tool execution
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
    pub error: Option<String>,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            success: false,
            content: String::new(),
            error: Some(error.into()),
        }
    }
}

/// Specification for a robot tool
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Trait for all robot tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool specification
    fn spec(&self) -> ToolSpec;

    /// Execute the tool with the given arguments
    async fn execute(&self, args: Value) -> ToolResult;
}
