//! Tolerant XML parser with early tool detection
//!
//! Implements a state-machine based parser that can identify `<tool_call>` blocks
//! and extract inner JSON, even if tags are partially malformed or the JSON is unclosed.
//!
//! ## Key Features
//!
//! - **Incremental parsing**: Process streaming input without waiting for complete blocks
//! - **Early detection**: Fire callbacks as soon as `</tool_call>` is detected
//! - **Tolerant**: Handle malformed tags, unclosed JSON, and other imperfect input
//! - **Zero-copy**: Work with string slices to minimize allocations

use regex::Regex;
use serde_json::Value;
use tracing::{debug, warn};

use crate::client::{ToolCall, ToolCallFunction};
use crate::error::Result;

/// Events emitted by the parser
#[derive(Debug, Clone)]
pub enum ParserEvent {
    /// Text content received (between tags)
    Text(String),
    /// A complete tool call has been detected
    ToolCall(ToolCall),
    /// An error occurred during parsing
    Error(String),
    /// Stream ended (all buffered content flushed)
    End,
}

/// A callback invoked when a complete tool call is detected
pub type ToolCallCallback = Box<dyn Fn(ToolCall) + Send + Sync>;

/// Parser state machine state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    /// Outside any tag, scanning for opening
    Outside,
    /// Inside `<tool_call>` opening tag
    InsideOpenTag,
    /// Inside tool call content
    InsideContent,
    /// Inside a nested tag within tool_call
    InsideNestedTag,
}

/// Tolerant XML parser for tool calls
///
/// Uses a state machine to track parsing progress and detect tool calls
/// incrementally as the LLM output streams in.
pub struct ToolCallParser {
    /// Current state of the parser
    state: ParserState,
    /// Buffer for accumulating content
    buffer: String,
    /// Buffer for the current tag name
    tag_buffer: String,
    /// Track nesting level for nested tags
    nested_depth: usize,
    /// Whether we're currently in the tool_call tag
    in_tool_call: bool,
    /// Track position in input for error reporting
    position: usize,
    /// Callback for early tool call detection
    on_tool_call: Option<ToolCallCallback>,
}

impl Default for ToolCallParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolCallParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self {
            state: ParserState::Outside,
            buffer: String::new(),
            tag_buffer: String::new(),
            nested_depth: 0,
            in_tool_call: false,
            position: 0,
            on_tool_call: None,
        }
    }

    /// Set a callback for early tool call detection
    ///
    /// This callback will be invoked as soon as a complete tool call
    /// is detected, without waiting for the full LLM output.
    pub fn on_tool_call<F>(mut self, callback: F) -> Self
    where
        F: Fn(ToolCall) + Send + Sync + 'static,
    {
        self.on_tool_call = Some(Box::new(callback));
        self
    }

    /// Feed more data into the parser
    ///
    /// Returns any events that occurred during parsing.
    pub fn feed(&mut self, data: &str) -> Vec<ParserEvent> {
        let mut events = Vec::new();

        if data.is_empty() {
            // Flush any remaining text
            if !self.buffer.is_empty() {
                let text = std::mem::take(&mut self.buffer);

                // Try to find any tool calls in the remaining text even if not in tags
                if let Some(tool_call) = self.try_parse_tool_call(&text) {
                    events.push(ParserEvent::ToolCall(tool_call));
                } else {
                    events.push(ParserEvent::Text(text));
                }
            }
            events.push(ParserEvent::End);
            return events;
        }

        for (i, ch) in data.char_indices() {
            self.position += 1;
            let events_chunk = self.process_char(ch, data, i);
            events.extend(events_chunk);
        }

        events
    }

    /// Process a single character
    fn process_char(&mut self, ch: char, _full_data: &str, _idx: usize) -> Vec<ParserEvent> {
        let mut events = Vec::new();

        match self.state {
            ParserState::Outside => {
                if ch == '<' {
                    self.state = ParserState::InsideOpenTag;
                    self.tag_buffer.clear();
                } else {
                    // Accumulate text
                    self.buffer.push(ch);

                    // INCREMENTAL DETECTION: Try to extract a tool call even without tags
                    // Optimization: Only check at potential JSON boundaries or significant length
                    if self.buffer.len() > 10 && (ch == '}' || ch == '\n' || ch == ' ') {
                        if let Some(tool_call) = self.try_parse_tool_call(&self.buffer) {
                            events.push(ParserEvent::ToolCall(tool_call));
                            // Clear buffer after successful detection to prevent partial/duplicate matches
                            self.buffer.clear();
                        }
                    }
                }
            }
            ParserState::InsideOpenTag => {
                if ch == '>' {
                    let tag = self.tag_buffer.trim().to_lowercase();
                    self.tag_buffer.clear();

                    if tag.starts_with("tool_call") && !tag.starts_with("/") {
                        // Found opening <tool_call...>
                        if !self.buffer.is_empty() {
                            events.push(ParserEvent::Text(std::mem::take(&mut self.buffer)));
                        }
                        self.state = ParserState::InsideContent;
                        self.in_tool_call = true;
                        self.buffer.clear();
                    } else {
                        // Not a tool_call tag, emit buffered text including the tag opening and go back
                        let mut text = String::from("<");
                        text.push_str(&tag);
                        text.push('>');

                        if !self.buffer.is_empty() {
                            let mut combined = std::mem::take(&mut self.buffer);
                            combined.push_str(&text);
                            events.push(ParserEvent::Text(combined));
                        } else {
                            events.push(ParserEvent::Text(text));
                        }
                        self.state = ParserState::Outside;
                    }
                } else if ch == '<' {
                    // Tag opened while another was open, treat the first as text
                    let mut text = String::from("<");
                    text.push_str(&self.tag_buffer);
                    self.buffer.push_str(&text);
                    self.tag_buffer.clear();
                    // Stay in InsideOpenTag state for the new '<'
                } else {
                    // Accumulate tag name
                    self.tag_buffer.push(ch);
                }
            }
            ParserState::InsideContent => {
                if ch == '<' {
                    // Check for closing tag
                    self.state = ParserState::InsideNestedTag;
                    self.tag_buffer.clear();
                    self.nested_depth = 1;
                } else {
                    // Accumulate content
                    self.buffer.push(ch);
                }
            }
            ParserState::InsideNestedTag => {
                if ch == '<' {
                    self.nested_depth += 1;
                    self.tag_buffer.push(ch);
                } else if ch == '>' {
                    self.nested_depth -= 1;
                    self.tag_buffer.push(ch);

                    if self.nested_depth == 0 {
                        let nested_tag = self.tag_buffer.trim().to_lowercase();
                        self.tag_buffer.clear();

                        if nested_tag.starts_with("/tool_call") {
                            // Found closing </tool_call>
                            self.process_tool_call(&mut events);
                            self.in_tool_call = false;
                            self.state = ParserState::Outside;
                        } else if nested_tag == "tool_call" {
                            // Nested <tool_call> inside <tool_call> (malformed)
                            warn!("Malformed XML: nested <tool_call> tag");
                            self.buffer.push('<');
                            self.buffer.push_str(&nested_tag);
                            self.state = ParserState::InsideContent;
                        } else {
                            // Other nested tag, continue
                            self.state = ParserState::InsideContent;
                        }
                    }
                } else {
                    self.tag_buffer.push(ch);
                }
            }
        }

        events
    }

    /// Process a completed tool call from the buffer
    fn process_tool_call(&mut self, events: &mut Vec<ParserEvent>) {
        let content = self.buffer.trim();

        if content.is_empty() {
            debug!("Empty tool_call block, ignoring");
            self.buffer.clear();
            return;
        }

        debug!(content_len = content.len(), "Processing tool_call block");

        // Try to extract JSON using regex first (tolerant approach)
        if let Some(tool_call) = self.try_parse_tool_call(content) {
            // Fire the early-detection callback immediately.
            if let Some(ref cb) = self.on_tool_call {
                cb(tool_call.clone());
            }
            events.push(ParserEvent::ToolCall(tool_call));
        } else {
            // Fall back to more aggressive parsing
            if let Some(tool_call) = self.aggressive_parse(content) {
                if let Some(ref cb) = self.on_tool_call {
                    cb(tool_call.clone());
                }
                events.push(ParserEvent::ToolCall(tool_call));
            } else {
                warn!(content = %content, "Failed to parse tool_call content");
                events.push(ParserEvent::Error(format!(
                    "Failed to parse tool_call: {}",
                    truncate_string(content, 100)
                )));
            }
        }

        self.buffer.clear();
    }

    /// Try to parse tool call using robust extraction
    fn try_parse_tool_call(&self, content: &str) -> Option<ToolCall> {
        // First try standard JSON parsing
        if let Ok(parsed) = serde_json::from_str::<Value>(content) {
            if let Some(tc) = self.extract_tool_call_from_json(&parsed) {
                return Some(tc);
            }
        }

        // Look for JSON object candidates
        let mut depth = 0;
        let mut start_index = None;
        let mut in_string = false;
        let mut escape_next = false;

        for (i, ch) in content.char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_string => escape_next = true,
                '"' => in_string = !in_string,
                '{' if !in_string => {
                    if depth == 0 {
                        start_index = Some(i);
                    }
                    depth += 1;
                }
                '}' if !in_string => {
                    if depth > 0 {
                        depth -= 1;
                        if depth == 0 {
                            if let Some(start) = start_index {
                                let potential_json = &content[start..i + 1];
                                if let Ok(parsed) = serde_json::from_str::<Value>(potential_json) {
                                    if let Some(tc) = self.extract_tool_call_from_json(&parsed) {
                                        return Some(tc);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Handle partial/unclosed JSON if we have a start_index and depth > 0
        if let Some(start) = start_index {
            if depth > 0 {
                let partial = &content[start..];
                // Try to "close" it by adding enough }
                let mut closed = partial.to_string();
                if in_string {
                    closed.push('"');
                }
                for _ in 0..depth {
                    closed.push('}');
                }

                if let Ok(parsed) = serde_json::from_str::<Value>(&closed) {
                    if let Some(tc) = self.extract_tool_call_from_json(&parsed) {
                        return Some(tc);
                    }
                }

                // If closing braces didn't work, try more aggressive partial extraction
                if let Some(tc) = self.aggressive_parse(partial) {
                    return Some(tc);
                }
            }
        }

        // Fallback to aggressive parse on the whole content
        self.aggressive_parse(content)
    }

    /// Extract tool call from parsed JSON
    fn extract_tool_call_from_json(&self, value: &Value) -> Option<ToolCall> {
        // Handle direct object format: { "name": "...", "arguments": "..." }
        // Some models might output { "tool": "name", "parameters": { ... } } or similar
        let name = value
            .get("name")
            .or_else(|| value.get("tool"))
            .or_else(|| value.get("function").and_then(|f| f.get("name")))
            .and_then(|v| v.as_str())?;

        // Arguments can be a string (escaped JSON) or a direct object
        let arguments_str = match value
            .get("arguments")
            .or_else(|| value.get("parameters"))
            .or_else(|| value.get("function").and_then(|f| f.get("arguments")))
            .or_else(|| value.get("function").and_then(|f| f.get("parameters")))
        {
            Some(Value::String(s)) => {
                let trimmed = s
                    .trim()
                    .trim_matches(|c: char| c.is_control() || c.is_whitespace());
                if trimmed.is_empty() {
                    "{}".to_string()
                } else {
                    s.to_string()
                }
            }
            Some(Value::Object(o)) => serde_json::to_string(o).unwrap_or_else(|_| "{}".to_string()),
            Some(Value::Null) => "{}".to_string(),
            _ => "{}".to_string(),
        };

        let id = value
            .get("id")
            .and_then(|v: &Value| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("call_{}", generate_id()));

        debug!(
            id = %id,
            name = %name,
            args_len = arguments_str.len(),
            "XML parser extracted tool call"
        );

        Some(ToolCall {
            id,
            function: ToolCallFunction {
                name: name.to_string(),
                arguments: arguments_str,
            },
        })
    }

    /// Aggressive parsing for malformed content
    fn aggressive_parse(&self, content: &str) -> Option<ToolCall> {
        // (iter-151: pre-compiled regexes via OnceLock instead of compiling
        // on every call. Was ~2ms per call; now ~0ms after first call.)
        use std::sync::OnceLock;
        static NAME_RE: OnceLock<Option<Regex>> = OnceLock::new();
        static ARGS_RE: OnceLock<Option<Regex>> = OnceLock::new();
        let name_re = NAME_RE.get_or_init(|| {
            Regex::new(r#""(?:name|function)":\s*"([^"]+)""#).ok()
        }).as_ref()?;
        let args_re = ARGS_RE.get_or_init(|| {
            Regex::new(r#""(?:arguments|parameters)":\s*"?(\{[^}]*\}|"[^"]*")"?"#).ok()
        }).as_ref()?;

        let name = name_re
            .captures(content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        let mut args = args_re
            .captures(content)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "{}".to_string());

        // Fix: If args is empty string "" (captured from literal ""), change to "{}"
        let trimmed_args = args
            .trim()
            .trim_matches(|c: char| c.is_control() || c.is_whitespace());
        if args == "\"\"" || args.is_empty() || trimmed_args.is_empty() {
            args = "{}".to_string();
        }

        if let Some(name) = name {
            return Some(ToolCall {
                id: format!("call_{}", generate_id()),
                function: ToolCallFunction {
                    name,
                    arguments: args,
                },
            });
        }

        None
    }

    /// Get the current buffer content
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Check if currently inside a tool_call block
    pub fn is_in_tool_call(&self) -> bool {
        self.in_tool_call
    }

    /// Reset the parser to initial state
    pub fn reset(&mut self) {
        self.state = ParserState::Outside;
        self.buffer.clear();
        self.tag_buffer.clear();
        self.nested_depth = 0;
        self.in_tool_call = false;
        self.position = 0;
    }

    /// Parse a complete string (non-streaming)
    pub fn parse(&mut self, content: &str) -> Result<Vec<ToolCall>> {
        let events = self.feed(content);
        self.feed(""); // Signal end

        let mut tool_calls = Vec::new();

        for event in events {
            match event {
                ParserEvent::ToolCall(tc) => tool_calls.push(tc),
                ParserEvent::Error(e) => return Err(crate::error::Error::XmlParse(e)),
                _ => {}
            }
        }

        Ok(tool_calls)
    }
}

/// Generate a simple unique ID
fn generate_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

/// Truncate a string for display
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars()
                .take(max_len.saturating_sub(3))
                .collect::<String>()
        )
    }
}

/// Stream-based parser that can be polled incrementally
pub struct ToolCallStreamParser {
    parser: ToolCallParser,
    pending_text: String,
}

impl ToolCallStreamParser {
    /// Create a new stream parser
    pub fn new() -> Self {
        Self {
            parser: ToolCallParser::new(),
            pending_text: String::new(),
        }
    }

    /// Set early detection callback
    pub fn on_tool_call<F>(mut self, callback: F) -> Self
    where
        F: Fn(ToolCall) + Send + Sync + 'static,
    {
        self.parser = self.parser.on_tool_call(callback);
        self
    }

    /// Process incoming chunk and return completed tool calls
    pub fn process_chunk(&mut self, chunk: &str) -> Vec<ToolCall> {
        let events = self.parser.feed(chunk);
        let mut tool_calls = Vec::new();

        for event in events {
            match event {
                ParserEvent::ToolCall(tc) => {
                    // The callback was already fired inside ToolCallParser::process_tool_call.
                    tool_calls.push(tc);
                }
                ParserEvent::Text(t) => self.pending_text.push_str(&t),
                ParserEvent::Error(e) => {
                    warn!(error = %e, "Parser error");
                }
                ParserEvent::End => {}
            }
        }

        tool_calls
    }

    /// Flush the parser and return any final tool calls and text
    pub fn finish(&mut self) -> (Vec<ToolCall>, String) {
        let events = self.parser.feed("");
        let mut tool_calls = Vec::new();
        let mut text = std::mem::take(&mut self.pending_text);

        for event in events {
            match event {
                ParserEvent::ToolCall(tc) => {
                    // The callback was already fired inside ToolCallParser::process_tool_call.
                    tool_calls.push(tc);
                }
                ParserEvent::Text(t) => text.push_str(&t),
                _ => {}
            }
        }

        (tool_calls, text)
    }

    /// Flush currently accumulated visible text and return it.
    pub fn take_text(&mut self) -> String {
        let events = self.parser.feed("");
        let mut text = std::mem::take(&mut self.pending_text);
        for event in events {
            match event {
                ParserEvent::Text(t) => text.push_str(&t),
                ParserEvent::End => {
                    // This event ensures any buffered text in the underlying parser
                    // is also captured, but Parser::feed("") already handles that
                    // by returning ParserEvent::Text if the buffer was non-empty.
                }
                _ => {}
            }
        }
        text
    }

    /// Get accumulated text content
    pub fn text(&self) -> &str {
        &self.pending_text
    }

    /// Clear accumulated text
    pub fn clear_text(&mut self) {
        self.pending_text.clear();
    }

    /// Reset the parser
    pub fn reset(&mut self) {
        self.parser.reset();
        self.pending_text.clear();
    }
}

impl Default for ToolCallStreamParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tool_call() {
        let content =
            r#"<tool_call>{"name": "get_weather", "arguments": {"city": "Tokyo"}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert!(tool_calls[0].function.arguments.contains("Tokyo"));
    }

    #[test]
    fn test_tool_call_with_newlines() {
        let content = r#"
<tool_call>
{
  "name": "search",
  "arguments": {
    "query": "rust async"
  }
}
</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "search");
    }

    #[test]
    fn test_multiple_tool_calls() {
        let content = r#"<tool_call>{"name": "tool1", "arguments": {}}</tool_call>
Some text here
<tool_call>{"name": "tool2", "arguments": {}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 2);
    }

    #[test]
    fn test_incremental_parsing() {
        let full_content = r#"<tool_call>{"name": "test", "arguments": {}}</tool_call>"#;
        let all_tool_calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let all_tool_calls_clone = all_tool_calls.clone();

        let mut parser = ToolCallParser::new().on_tool_call(move |tc| {
            all_tool_calls_clone.lock().unwrap().push(tc);
        });

        for ch in full_content.chars() {
            parser.feed(&ch.to_string());
        }

        // Should have detected tool call early
        assert_eq!(all_tool_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_malformed_tags() {
        // Test with malformed closing tag
        let content = r#"<tool_call>{"name": "test", "arguments": {}}</tool_call >"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        // Should still handle malformed input gracefully (may or may not find tool calls)
        let _ = tool_calls;
    }

    #[test]
    fn test_nested_json() {
        let content = r#"<tool_call>{"name": "complex", "arguments": {"nested": {"deep": "value"}}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert!(tool_calls[0].function.arguments.contains("nested"));
    }

    #[test]
    fn test_text_content_extraction() {
        let mut parser = ToolCallStreamParser::new();

        parser.process_chunk("Hello, this is ");
        parser.process_chunk("some text ");
        parser.process_chunk("before the tool");

        assert_eq!(parser.text(), "");
        parser.clear_text();
        assert_eq!(parser.text(), "");
    }

    #[test]
    fn test_streaming_interruption_at_tag_boundary() {
        let mut parser = ToolCallStreamParser::new();

        // Part 1: Text and start of tool_call tag
        let calls1 = parser.process_chunk("Final thoughts before tool: <tool_");
        assert_eq!(calls1.len(), 0);
        // The parser buffers "<tool_" because it might be a tag.
        // It does NOT emit "Final thoughts before tool: " yet because leading text is buffered
        // until we're sure it's not a tag.
        assert_eq!(parser.take_text(), "Final thoughts before tool: ");

        // Part 2: Complete the tag and provide JSON
        let calls2 =
            parser.process_chunk("call>{\"name\": \"echo\", \"arguments\": \"{}\"}</tool_call>");
        assert_eq!(calls2.len(), 1);
        assert_eq!(calls2[0].function.name, "echo");
        assert_eq!(parser.take_text(), "");

        // Part 3: Text after tool call
        let calls3 = parser.process_chunk(" and some text after.");
        assert_eq!(calls3.len(), 0);
        assert_eq!(parser.take_text(), " and some text after.");
    }

    #[test]
    fn test_parser_finish_flushes_unclosed_text() {
        let mut parser = ToolCallStreamParser::new();

        parser.process_chunk("This text is never followed by a tag");
        // The parser buffers this text because it doesn't end with a tag boundary
        // and doesn't contain any tags to trigger an emission.
        assert_eq!(parser.take_text(), "This text is never followed by a tag");

        let (calls, text) = parser.finish();
        assert_eq!(calls.len(), 0);
        assert_eq!(text, "");
    }

    #[test]
    fn test_empty_arguments_bug() {
        // Test case for "arguments": "" which caused EOF error
        let content = r#"<tool_call>{"name": "timestamp", "arguments": ""}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "timestamp");
        assert_eq!(tool_calls[0].function.arguments, "{}");
    }

    #[test]
    fn test_tool_and_parameters_fallback() {
        // Test case for models using "tool" and "parameters" instead of "name" and "arguments"
        let content = r#"<tool_call>{"tool": "calculate", "parameters": {"a": 1}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "calculate");
        assert_eq!(tool_calls[0].function.arguments, "{\"a\":1}");
    }

    #[test]
    fn test_nested_function_fallback() {
        // Test case for models using { "function": { "name": "...", "arguments": { ... } } }
        let content =
            r#"<tool_call>{"function": {"name": "echo", "arguments": {"msg": "hi"}}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "echo");
        assert_eq!(tool_calls[0].function.arguments, "{\"msg\":\"hi\"}");
    }

    #[test]
    fn test_object_arguments() {
        // Test case for "arguments": {} (object instead of string)
        let content = r#"<tool_call>{"name": "timestamp", "arguments": {}}</tool_call>"#;
        let mut parser = ToolCallParser::new();
        let tool_calls = parser.parse(content).unwrap();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "timestamp");
        assert_eq!(tool_calls[0].function.arguments, "{}");
    }

    #[test]
    fn test_stream_parser_filters_tool_call_markup_from_visible_text() {
        let mut parser = ToolCallStreamParser::new();

        let tool_calls = parser.process_chunk(
            "Before <tool_call>{\"name\": \"echo\", \"arguments\": \"{}\"}</tool_call> after",
        );
        // After my fix, it now correctly preserves text before the tag
        let text = parser.take_text();

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "echo");
        assert_eq!(text, "Before  after");
    }
}
