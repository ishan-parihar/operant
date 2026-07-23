//! Learning graph mutation tool — wires up `learning_graph::{delete_node, edit_node}`
//! as an LLM-callable tool.
//!
//! Ported from hermes-agent's `agent/learning_mutations.py`.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent::learning_graph::{delete_node, edit_node};
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Arguments for the `learning_manage` tool.
#[derive(Debug, Clone, JsonSchema, Deserialize)]
pub struct LearningManageArgs {
    /// The action to perform: "delete" or "edit".
    pub action: String,
    /// The node ID to operate on (e.g. "skill:my-skill" or "memory:MEMORY.md:3").
    pub node_id: String,
    /// The new content (required for "edit" action, ignored for "delete").
    #[serde(default)]
    pub content: Option<String>,
}

/// Tool that manages learning graph nodes (skills and memories).
pub struct LearningMutationTool {
    skills_dir: std::path::PathBuf,
    memory_dir: std::path::PathBuf,
}

impl LearningMutationTool {
    pub fn new(skills_dir: std::path::PathBuf, memory_dir: std::path::PathBuf) -> Self {
        Self {
            skills_dir,
            memory_dir,
        }
    }
}

#[async_trait]
impl OperantTool for LearningMutationTool {
    fn name(&self) -> &str {
        "learning_manage"
    }

    fn description(&self) -> &str {
        "Manage the learning graph by deleting or editing skill and memory nodes. \
         Use this to remove outdated skills/memories or update their content."
    }

    fn is_available(&self) -> bool {
        self.skills_dir.exists() && self.memory_dir.exists()
    }

    fn schema(&self) -> crate::tools::ToolSchema {
        crate::tools::ToolSchema {
            name: "learning_manage".to_string(),
            description: "Manage learning graph nodes (skills and memories)".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["delete", "edit"],
                        "description": "The action to perform"
                    },
                    "node_id": {
                        "type": "string",
                        "description": "Node ID (e.g. 'skill:my-skill' or 'memory:MEMORY.md:3')"
                    },
                    "content": {
                        "type": "string",
                        "description": "New content (required for edit, ignored for delete)"
                    }
                },
                "required": ["action", "node_id"]
            }),
        }
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        // Validate directories exist
        if !self.skills_dir.exists() {
            return ToolResult::error(
                "learning_manage",
                format!("Skills directory not found: {}", self.skills_dir.display()),
            );
        }
        if !self.memory_dir.exists() {
            return ToolResult::error(
                "learning_manage",
                format!("Memory directory not found: {}", self.memory_dir.display()),
            );
        }

        let parsed: LearningManageArgs = match serde_json::from_value(args) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::error(
                    "learning_manage",
                    format!("Invalid arguments: {}", e),
                );
            }
        };

        let result = match parsed.action.as_str() {
            "delete" => delete_node(
                &parsed.node_id,
                &self.skills_dir,
                &self.memory_dir,
            ),
            "edit" => {
                let content = match parsed.content {
                    Some(c) => c,
                    None => {
                        return ToolResult::error(
                            "learning_manage",
                            "content is required for edit action",
                        );
                    }
                };
                edit_node(
                    &parsed.node_id,
                    &content,
                    &self.skills_dir,
                    &self.memory_dir,
                )
            }
            other => {
                return ToolResult::error(
                    "learning_manage",
                    format!("Unknown action: {}", other),
                );
            }
        };

        let payload = json!({
            "ok": result.ok,
            "message": result.message
        });
        if result.ok {
            ToolResult::success("learning_manage", payload)
        } else {
            ToolResult::error("learning_manage", result.message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_schema_valid() {
        let tool = LearningMutationTool::new(
            std::path::PathBuf::from("/tmp/skills"),
            std::path::PathBuf::from("/tmp/memory"),
        );
        let schema = tool.schema();
        assert_eq!(schema.name, "learning_manage");
        assert!(schema.description.contains("learning graph"));
    }

    #[test]
    fn test_delete_nonexistent_node() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        let memory_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&memory_dir).unwrap();

        let result = delete_node("skill:nonexistent", &skills_dir, &memory_dir);
        assert!(!result.ok);
        assert!(result.message.contains("not found") || result.message.contains("does not exist"));
    }

    #[test]
    fn test_delete_node_invalid_id() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");
        let memory_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::create_dir_all(&memory_dir).unwrap();

        let result = delete_node("invalid_id", &skills_dir, &memory_dir);
        assert!(!result.ok);
    }
}
