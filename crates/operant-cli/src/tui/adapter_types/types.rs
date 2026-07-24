pub mod types {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Role {
        User,
        Assistant,
        System,
    }

    /// Rich content block variants used by the TUI transcript renderer.
    ///
    /// Each variant corresponds to a distinct visual element in the conversation
    /// view.  These are NOT LLM API types — they are purely display-layer concepts
    /// assembled from `AgentEvent` values in `App::handle_agent_event`.
    #[derive(Debug, Clone)]
    pub enum ContentBlock {
        Text {
            text: String,
        },
        Thinking {
            thinking: String,
            #[allow(dead_code)] // Signature field for thinking blocks
            signature: String,
        },
        RedactedThinking {
            data: String,
        },
        ToolUse {
            id: String,
            name: String,
            input: serde_json::Value,
        },
        ToolResult {
            tool_use_id: String,
            content: ToolResultContent,
            is_error: bool,
        },
        Image {
            source: String,
            data: String,
            media_type: String,
        },
        Document {
            title: String,
            context: String,
            source: String,
        },
        UserLocalCommandOutput {
            command: String,
            output: String,
        },
        UserCommand {
            name: String,
            args: String,
        },
        UserMemoryInput {
            key: String,
            value: String,
        },
        SystemAPIError {
            message: String,
            retry_secs: Option<u64>,
        },
        CollapsedReadSearch {
            tool_name: String,
            paths: Vec<String>,
            n_hidden: usize,
        },
        TaskAssignment {
            id: String,
            subject: String,
            description: String,
        },
    }

    #[derive(Debug, Clone)]
    pub enum ToolResultContent {
        Text(String),
        Image { data: String, media_type: String },
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub enum MessageContent {
        Text(String),
        Blocks(Vec<ContentBlock>),
    }

    #[derive(Debug, Clone)]
    pub struct Message {
        pub role: Role,
        pub content: MessageContent,
    }

    impl Message {
        #[allow(dead_code)] // User message constructor
        pub fn user(text: String) -> Self {
            Self {
                role: Role::User,
                content: MessageContent::Text(text),
            }
        }
        pub fn assistant(text: String) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Text(text),
            }
        }
        pub fn assistant_blocks(blocks: Vec<ContentBlock>) -> Self {
            Self {
                role: Role::Assistant,
                content: MessageContent::Blocks(blocks),
            }
        }
        pub fn content_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks.iter().collect(),
                MessageContent::Text(_) => vec![],
            }
        }
        pub fn text_content(&self) -> String {
            match &self.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            }
        }
        pub fn get_all_text(&self) -> String {
            self.text_content()
        }
        pub fn get_tool_use_blocks(&self) -> Vec<&ContentBlock> {
            match &self.content {
                MessageContent::Blocks(blocks) => blocks
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
                    .collect(),
                _ => vec![],
            }
        }
    }
}

