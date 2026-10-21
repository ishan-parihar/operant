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
///
/// The enum is a WIRE vocabulary: every variant and field is deserialized from
/// adapter JSON, so `dead_code` is expected in production (the variants are
/// parsed, not built) AND in the test profile (the tests parse the payloads
/// without reading every field back). Two cfg arms, one reason each — a single
/// `#[cfg_attr(not(test), expect(...))]` silences the production profile only and
/// leaves the test-profile warnings standing, which is what `--all-targets`
/// was reporting for this file.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "AD-034: adapter wire enum; variants are parsed from adapter JSON, not built here"
    )
)]
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "AD-034: same wire vocabulary; the test profile parses the payloads without reading every field back"
    )
)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[cfg_attr(
            not(test),
            expect(
                dead_code,
                reason = "AD-034: deserialized from the adapter wire format; kept so payloads round-trip"
            )
        )]
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
        #[cfg_attr(
            not(test),
            expect(
                dead_code,
                reason = "AD-034: deserialized from the adapter wire format; kept so payloads round-trip"
            )
        )]
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
#[expect(
    dead_code,
    reason = "AD-034: adapter wire enum; variants are parsed from adapter JSON, not built here"
)]
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
    #[cfg_attr(not(test), expect(dead_code, reason = "AD-034: test-only constructor"))]
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
    #[cfg_attr(not(test), expect(dead_code, reason = "AD-034: test-only accessor"))]
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
}
