pub mod cost {
    #[derive(Debug, Clone, Default)]
    pub struct CostTracker {
        pub total_cost: f64,
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub model: String,
    }

    impl CostTracker {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn record_usage(&mut self, input: u32, output: u32) {
            self.input_tokens += input;
            self.output_tokens += output;
        }
        /// Accumulate a real per-request cost (from `AgentEvent::Cost`'s
        /// models_dev-sourced estimate, or a flat-rate fallback when the
        /// model isn't in the models_dev catalog).
        pub fn record_cost(&mut self, cost_usd: f64) {
            self.total_cost += cost_usd;
        }
        pub fn set_model(&mut self, model: &str) {
            self.model = model.to_string();
        }
    }
}

// (iter-209: pub mod file_history { ... } deleted — stub where
// snapshots_for_turn always returned vec![] and latest_turn_index
// returned None. The /changes turn-diff feature never worked.
// Removed: FileHistory, FileSnapshot, App.file_history field,
// App.attach_turn_diff_state, refresh_turn_diff_from_history,
// build_turn_diff in diff_viewer.rs. The diff_viewer's real git-diff
// functionality (/diff, /review) is unaffected.
// To re-implement: wire to operant_core::tools::file_state or a new
// per-turn snapshot store in core.)

// (iter-136: ImageSource enum deleted — zero callers, ponytail-audit Tier-2 cut)

// (iter-165: keybindings module deleted — completely unused after iter-164)

/// TUI-side rich rendering types.
///
/// # Why this module is intentionally separate from `operant_core::client`
///
/// There are two distinct message type families in this codebase and they must
/// NOT be merged:
///
/// - **`operant_core::client::Message`** — the wire-format LLM API message.
///   `content: String`, `reasoning: Option<String>`, `tool_calls: Option<Vec<ToolCall>>`.
///   Serialized to/from JSON for OpenAI/Anthropic API calls.  Lives in operant-core
///   and must remain free of TUI/rendering concerns.
///
/// - **`adapter_types::types::Message`** (this module) — the TUI rendering message.
///   `content: MessageContent` which holds rich `ContentBlock` variants: Text, Thinking,
///   ToolUse, ToolResult, Image, Document, UserLocalCommandOutput, etc.  Used exclusively
///   by the ratatui transcript renderer in `render/`.  These variants do not correspond
///   1-to-1 with any LLM API concept.
///
/// # How the two domains connect
///
/// `App::handle_agent_event` in `tui/app.rs` is the bridge.  It receives
/// `AgentEvent` variants emitted by the core agent loop and maps them into
/// TUI `ContentBlock`s:
///
/// - `AgentEvent::Content { text }` → accumulate into `streaming_text`, eventually
///   flushed as `ContentBlock::Text`.
/// - `AgentEvent::Thinking { content }` → `ContentBlock::Thinking { thinking, signature }`.
/// - `AgentEvent::ToolStart { .. }` → `ToolUseBlock` tracked in `app.tool_use_blocks`.
/// - `AgentEvent::ToolComplete { result }` → status update on the `ToolUseBlock`.
/// - `AgentEvent::Done { message }` → if no streamed content, `message.content: String`
///   and `message.reasoning: Option<String>` (core wire fields) are mapped to
///   `ContentBlock::Text` / `ContentBlock::Thinking` and pushed as a TUI `Message`.
///
/// This mapping is the correct and intentional seam.  Do NOT replace the TUI `Message`
/// with `operant_core::client::Message`; that would destroy the transcript rendering.
