use crate::schema::ToolSchema;

/// A single entry in conversation context, representing a message with role and content.
pub struct ContextEntry {
    pub role: String,
    pub content: String,
}

/// Constructs system and user prompts from structured parts (tool schemas, instructions, context).
#[derive(Debug)]
pub struct PromptBuilder {
    system_preamble: String,
    tool_schemas: String,
    max_tokens: usize,
}

impl PromptBuilder {
    /// Creates a new PromptBuilder with an empty preamble, no tool schemas, and default max_tokens of 4096.
    pub fn new() -> Self {
        Self {
            system_preamble: String::new(),
            tool_schemas: String::new(),
            max_tokens: 4096,
        }
    }

    /// Sets the system preamble prepended to every system prompt.
    pub fn with_preamble(mut self, preamble: impl Into<String>) -> Self {
        self.system_preamble = preamble.into();
        self
    }

    /// Registers tool schemas, serializing them to JSON for inclusion in system prompts.
    pub fn with_tool_schemas(mut self, schemas: &[ToolSchema]) -> Self {
        self.tool_schemas = serde_json::to_string(schemas).unwrap_or_default();
        self
    }

    /// Sets the token budget hint for prompt construction.
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Builds the system prompt with preamble, tool schemas (if any), and ReAct format instructions.
    pub fn build_system_prompt(&self) -> String {
        let mut prompt = String::new();

        if !self.system_preamble.is_empty() {
            prompt.push_str(&self.system_preamble);
            prompt.push_str("\n\n");
        }

        if !self.tool_schemas.is_empty() {
            prompt.push_str("Available tools:\n");
            prompt.push_str(&self.tool_schemas);
            prompt.push_str("\n\n");
        }

        prompt.push_str(
            "Use these tools when appropriate. Respond with JSON following the ReAct format:\n\
             Thought: ...\n\
             Action: tool_name\n\
             ActionInput: {\"arg\": \"value\"}",
        );

        format!("{}\n\nMax response tokens: {}", prompt, self.max_tokens)
    }

    /// Builds a user prompt from a message and optional context history.
    pub fn build_user_prompt(&self, message: &str, context: &[ContextEntry]) -> String {
        if context.is_empty() {
            return message.to_string();
        }

        let mut prompt = String::from("Previous conversation:\n");
        for entry in context {
            prompt.push_str(&format!("[{}]: {}\n", entry.role, entry.content));
        }
        prompt.push_str("\nCurrent message:\n");
        prompt.push_str(message);

        prompt
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[allow(dead_code)]
    #[derive(JsonSchema, Deserialize)]
    struct TestToolArgs {
        query: String,
        limit: Option<i32>,
    }

    #[test]
    fn test_new_prompt_builder_defaults() {
        let pb = PromptBuilder::new();
        assert!(pb.system_preamble.is_empty());
        assert!(pb.tool_schemas.is_empty());
        assert_eq!(pb.max_tokens, 4096);
    }

    #[test]
    fn test_build_system_prompt_with_schemas() {
        let schema =
            ToolSchema::from_type::<TestToolArgs>("search_tool", "Searches for information");
        let pb = PromptBuilder::new()
            .with_preamble("You are Operant.")
            .with_tool_schemas(&[schema]);

        let prompt = pb.build_system_prompt();
        assert!(prompt.contains("You are Operant."));
        assert!(prompt.contains("Available tools:"));
        assert!(prompt.contains("search_tool"));
        assert!(prompt.contains("ReAct format"));
        assert!(prompt.contains("Thought:"));
        assert!(prompt.contains("Action:"));
        assert!(prompt.contains("ActionInput:"));
    }

    #[test]
    fn test_build_user_prompt_basic() {
        let pb = PromptBuilder::new();
        let prompt = pb.build_user_prompt("Hello", &[]);
        assert_eq!(prompt, "Hello");
    }

    #[test]
    fn test_build_user_prompt_with_context() {
        let pb = PromptBuilder::new();
        let context = vec![
            ContextEntry {
                role: "user".to_string(),
                content: "What time is it?".to_string(),
            },
            ContextEntry {
                role: "assistant".to_string(),
                content: "Let me check.".to_string(),
            },
        ];
        let prompt = pb.build_user_prompt("Thanks!", &context);

        assert!(prompt.contains("Previous conversation:"));
        assert!(prompt.contains("[user]: What time is it?"));
        assert!(prompt.contains("[assistant]: Let me check."));
        assert!(prompt.contains("Current message:"));
        assert!(prompt.contains("Thanks!"));
    }

    #[test]
    fn test_prompt_builder_with_max_tokens() {
        let pb = PromptBuilder::new().with_max_tokens(8192);
        assert_eq!(pb.max_tokens, 8192);
    }
}
