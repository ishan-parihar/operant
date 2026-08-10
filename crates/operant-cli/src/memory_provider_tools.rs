//! Register the memory provider's own tool schemas (`memory_smart_search`,
//! `memory_save`, ...) into the `ToolRegistry` — hermes-plugin parity.
//!
//! The agentmemory MCP server is injected as a **deferred** server (it is
//! not auto-connected at startup, so an operant invocation never spawns
//! `npx @agentmemory/mcp` unless the user actually connects it). These
//! provider-backed tools keep the memory surface available to the model
//! without the MCP server, exactly like the hermes-agent plugin registers
//! its memory tools independently of the optional MCP server.

use std::sync::Arc;

use async_trait::async_trait;
use operant_core::memory_provider::MemoryProvider;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};
use serde_json::Value;

/// Adapter exposing a `MemoryProvider` tool schema as an `OperantTool`.
struct MemoryProviderTool {
    name: String,
    description: String,
    parameters: Value,
    provider: Arc<dyn MemoryProvider>,
}

#[async_trait]
impl OperantTool for MemoryProviderTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(&self.name, &self.description, self.parameters.clone())
    }

    fn toolset(&self) -> &str {
        "memory"
    }

    fn is_available(&self) -> bool {
        self.provider.is_available()
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        // handle_tool_call returns a JSON result string; surface it verbatim.
        let content = self.provider.handle_tool_call(&self.name, args).await;
        ToolResult {
            tool_call_id: String::new(),
            name: self.name.clone(),
            success: true,
            content,
            error: None,
        }
    }
}

/// Register every tool schema advertised by the memory provider.
///
/// Best-effort: a malformed schema is skipped, never fatal to startup.
pub async fn register_provider_tools(registry: &ToolRegistry, provider: Arc<dyn MemoryProvider>) {
    for schema in provider.tool_schemas() {
        let Some(name) = schema.get("name").and_then(Value::as_str) else {
            continue;
        };
        let description = schema
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parameters = schema.get("parameters").cloned().unwrap_or_else(|| {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true,
            })
        });
        let tool = MemoryProviderTool {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
            provider: provider.clone(),
        };
        if let Err(e) = registry.register(tool).await {
            tracing::warn!(
                tool = name,
                error = %e,
                "Failed to register memory provider tool"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::memory_provider::MemoryProvider;
    use serde_json::json;

    /// Minimal provider that advertises one memory tool.
    struct StubProvider;

    #[async_trait]
    impl MemoryProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn initialize(&self, _session_id: &str) -> operant_core::error::Result<()> {
            Ok(())
        }

        fn tool_schemas(&self) -> Vec<Value> {
            vec![json!({
                "name": "memory_stub_search",
                "description": "Stub search",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            })]
        }

        async fn handle_tool_call(&self, _name: &str, args: Value) -> String {
            serde_json::json!({ "ok": true, "query": args.get("query") }).to_string()
        }
    }

    #[tokio::test]
    async fn registers_provider_tool_schema() {
        let registry = ToolRegistry::new(std::time::Duration::from_secs(10));
        register_provider_tools(&registry, Arc::new(StubProvider)).await;

        let schemas = registry.get_schemas().await;
        let stub = schemas
            .iter()
            .find(|s| s.name == "memory_stub_search")
            .expect("provider tool must be registered");
        assert_eq!(stub.description, "Stub search");
        assert_eq!(stub.parameters["properties"]["query"]["type"], "string");

        // Execute dispatches through the provider.
        let result = registry
            .execute(
                "memory_stub_search",
                "test-call",
                json!({ "query": "hello" }),
                ToolContext::default(),
            )
            .await
            .expect("tool execution must succeed");
        assert!(result.success);
        assert!(result.content.contains("hello"));
    }
}
