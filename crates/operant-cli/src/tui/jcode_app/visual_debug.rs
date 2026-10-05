// Vendored from jcode (crates/jcode-tui-visual-debug/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Partial port @ 0a9dc7805: the capture
// TYPE family (plain data structs), FrameCaptureBuilder (new/anomaly/check/
// build), and check_shift_enter_anomaly — exactly the surface ui_input's draw
// path consumes. The rest of the upstream crate (frame buffer, capture sinks,
// serialization writers — the 857-line debug tooling surface) stays out per
// the plan's visual-debug skip; operant has debug/overlay.rs for that surface.

use ratatui::layout::Rect;
use serde::Serialize;
use serde_json::Value;

/// A captured frame with all render context
#[derive(Debug, Clone, Serialize)]
pub struct FrameCapture {
    /// Frame number (monotonically increasing)
    pub frame_id: u64,
    /// Timestamp when frame was rendered
    pub timestamp: std::time::SystemTime,
    /// Terminal dimensions
    pub terminal_size: (u16, u16),
    /// Layout areas computed for this frame
    pub layout: LayoutCapture,
    /// State snapshot at render time
    pub state: StateSnapshot,
    /// Any anomalies detected during rendering
    pub anomalies: Vec<String>,
    /// The actual text content rendered to each area (stripped of ANSI)
    pub rendered_text: RenderedText,
    /// Mermaid image regions detected in wrapped content
    pub image_regions: Vec<ImageRegionCapture>,
    /// Render timing information (milliseconds)
    pub render_timing: Option<RenderTimingCapture>,
    /// Info widget placements and summary data
    pub info_widgets: Option<InfoWidgetCapture>,
    /// Render order for major phases
    pub render_order: Vec<String>,
    /// Mermaid debug stats snapshot (if available)
    pub mermaid: Option<Value>,
    /// Side-panel debug snapshot, including live Mermaid utilization when available
    pub side_panel: Option<Value>,
    /// Markdown debug stats snapshot (if available)
    pub markdown: Option<Value>,
    /// Theme/palette snapshot (if available)
    pub theme: Option<Value>,
}

/// Captured layout computation
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LayoutCapture {
    /// Whether packed layout was used (vs scrolling)
    pub use_packed: bool,
    /// Estimated content height
    pub estimated_content_height: usize,
    /// Messages area
    pub messages_area: Option<RectCapture>,
    /// Diagram area (pinned diagram pane)
    pub diagram_area: Option<RectCapture>,
    /// Status line area
    pub status_area: Option<RectCapture>,
    /// Queued messages area
    pub queued_area: Option<RectCapture>,
    /// Input area
    pub input_area: Option<RectCapture>,
    /// Input line count (before wrapping)
    pub input_lines_raw: usize,
    /// Input line count (after wrapping)
    pub input_lines_wrapped: usize,
    /// Margin widths for info widgets (per visible row)
    pub margins: Option<MarginsCapture>,
    /// Info widget placements
    pub widget_placements: Vec<WidgetPlacementCapture>,
}

/// Rect capture (serializable)
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct RectCapture {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Margin widths captured for debug
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MarginsCapture {
    pub left_widths: Vec<u16>,
    pub right_widths: Vec<u16>,
    pub centered: bool,
}

/// Info widget placement capture
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WidgetPlacementCapture {
    pub kind: String,
    pub side: String,
    pub rect: RectCapture,
}

/// Render timing capture (milliseconds)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RenderTimingCapture {
    pub prepare_ms: f32,
    pub draw_ms: f32,
    pub total_ms: f32,
    pub messages_ms: Option<f32>,
    pub widgets_ms: Option<f32>,
}

/// Info widget summary capture
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct InfoWidgetSummary {
    pub todos_total: usize,
    pub todos_done: usize,
    pub context_total_chars: Option<usize>,
    pub context_limit: Option<usize>,
    pub queue_mode: Option<bool>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub session_count: Option<usize>,
    pub client_count: Option<usize>,
    pub memory_total: Option<usize>,
    pub memory_project: Option<usize>,
    pub memory_global: Option<usize>,
    pub memory_activity: Option<bool>,
    pub swarm_session_count: Option<usize>,
    pub swarm_member_count: Option<usize>,
    pub swarm_subagent_status: Option<String>,
    pub background_running: Option<usize>,
    pub background_tasks: Option<usize>,
    pub usage_available: Option<bool>,
    pub usage_provider: Option<String>,
    pub tokens_per_second: Option<f32>,
    pub auth_method: Option<String>,
    pub upstream_provider: Option<String>,
}

/// Info widget capture (summary + placements)
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct InfoWidgetCapture {
    pub summary: InfoWidgetSummary,
    pub placements: Vec<WidgetPlacementCapture>,
}

impl From<Rect> for RectCapture {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// State snapshot at render time
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StateSnapshot {
    pub is_processing: bool,
    pub input_len: usize,
    pub input_preview: String,
    pub cursor_pos: usize,
    pub scroll_offset: usize,
    pub queued_count: usize,
    pub message_count: usize,
    pub streaming_text_len: usize,
    pub has_suggestions: bool,
    pub status: String,
    pub diagram_mode: Option<String>,
    pub diagram_focus: bool,
    pub diagram_index: usize,
    pub diagram_count: usize,
    pub diagram_scroll_x: i32,
    pub diagram_scroll_y: i32,
    pub diagram_pane_ratio: u8,
    pub diagram_pane_enabled: bool,
    pub diagram_pane_position: Option<String>,
    pub diagram_zoom: u8,
}

/// Actual rendered text content
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RenderedText {
    /// Status line text (spinner, tokens, elapsed, etc.)
    pub status_line: String,
    /// Input area text (what the user is typing)
    pub input_area: String,
    /// Hint text shown above input (if any)
    pub input_hint: Option<String>,
    /// Queued messages (messages waiting to be sent)
    pub queued_messages: Vec<String>,
    /// Recent messages displayed (last few for context)
    pub recent_messages: Vec<MessageCapture>,
    /// Streaming text (if currently streaming)
    pub streaming_text_preview: String,
}

/// Mermaid image region capture
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ImageRegionCapture {
    pub hash: String,
    pub abs_line_idx: usize,
    pub height: u16,
}

/// Captured message for debugging
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MessageCapture {
    pub role: String,
    pub content_preview: String,
    pub content_len: usize,
}

// [port-excision] upstream FrameBuffer (the ring buffer of recent frames
// behind get_frame_buffer) is not part of this partial — it feeds the capture
// sinks/writers the plan skips. ui_input's draw path never touches it.

/// Builder for constructing frame captures during rendering
#[derive(Default)]
pub struct FrameCaptureBuilder {
    pub layout: LayoutCapture,
    pub state: StateSnapshot,
    pub rendered_text: RenderedText,
    pub image_regions: Vec<ImageRegionCapture>,
    pub anomalies: Vec<String>,
    pub render_timing: Option<RenderTimingCapture>,
    pub info_widgets: Option<InfoWidgetCapture>,
    pub render_order: Vec<String>,
    pub mermaid: Option<Value>,
    pub side_panel: Option<Value>,
    pub markdown: Option<Value>,
    pub theme: Option<Value>,
    terminal_size: (u16, u16),
}

impl FrameCaptureBuilder {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            terminal_size: (width, height),
            ..Default::default()
        }
    }

    /// Record an anomaly detected during rendering
    pub fn anomaly(&mut self, msg: impl Into<String>) {
        self.anomalies.push(msg.into());
    }

    /// Check a condition and record anomaly if false
    pub fn check(&mut self, condition: bool, msg: impl Into<String>) {
        if !condition {
            self.anomalies.push(msg.into());
        }
    }

    /// Build the final frame capture
    pub fn build(self) -> FrameCapture {
        FrameCapture {
            frame_id: 0, // Will be set by buffer
            timestamp: std::time::SystemTime::now(),
            terminal_size: self.terminal_size,
            layout: self.layout,
            state: self.state,
            anomalies: self.anomalies,
            rendered_text: self.rendered_text,
            image_regions: self.image_regions,
            render_timing: self.render_timing,
            info_widgets: self.info_widgets,
            render_order: self.render_order,
            mermaid: self.mermaid,
            side_panel: self.side_panel,
            markdown: self.markdown,
            theme: self.theme,
        }
    }
}

/// Check for the specific alternate-send hint anomaly.
pub fn check_shift_enter_anomaly(
    builder: &mut FrameCaptureBuilder,
    is_processing: bool,
    input_text: &str,
    hint_shown: bool,
) {
    // The hint should ONLY show when processing AND input is non-empty
    let should_show = is_processing && !input_text.is_empty();

    if hint_shown != should_show {
        builder.anomaly(format!(
            "alternate-send hint mismatch: shown={}, should_show={} (is_processing={}, input_len={})",
            hint_shown,
            should_show,
            is_processing,
            input_text.len()
        ));
    }

    // Also check if the hint text appears in the input itself (the bug!)
    if input_text.to_lowercase().contains("shift") && input_text.to_lowercase().contains("enter") {
        builder.anomaly(format!(
            "INPUT CONTAINS 'shift'+'enter' - possible hint leak: {:?}",
            input_text
        ));
    }
}
