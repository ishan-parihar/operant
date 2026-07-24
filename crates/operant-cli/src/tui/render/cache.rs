// render/cache.rs — Rendered line items, message/completion/streaming caches.

use std::cell::RefCell;

use crate::tui::virtual_list::VirtualItem;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

#[derive(Clone)]
pub(crate) struct RenderedLineItem {
    pub(crate) line: Line<'static>,
    pub(crate) search_text: String,
    pub(crate) is_header: bool,
    pub(crate) message_index: Option<usize>,
    /// If this line is the clickable header of a thinking block, its hash.
    pub(crate) thinking_hash: Option<u64>,
}

impl VirtualItem for RenderedLineItem {
    fn measure_height(&self, _width: u16) -> u16 {
        1
    }

    fn render(&self, area: Rect, buf: &mut Buffer, _selected: bool) {
        Paragraph::new(vec![self.line.clone()]).render(area, buf);
    }

    fn search_text(&self) -> String {
        self.search_text.clone()
    }

    fn is_section_header(&self) -> bool {
        self.is_header
    }
}

pub(crate) fn flatten_line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageLinesCacheKey {
    pub(crate) width: u16,
    pub(crate) transcript_version: u64,
    pub(crate) messages_ptr: usize,
    pub(crate) messages_len: usize,
    pub(crate) annotations_ptr: usize,
    pub(crate) annotations_len: usize,
    pub(crate) thinking_expanded_len: usize,
}

#[derive(Clone)]
pub(crate) struct MessageLinesCache {
    pub(crate) key: MessageLinesCacheKey,
    pub(crate) lines: Vec<RenderedLineItem>,
}

/// Cache key for completed messages only (no ptr — len change = new message).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletedMsgCacheKey {
    pub(crate) width: u16,
    pub(crate) transcript_version: u64,
    pub(crate) messages_len: usize,
    pub(crate) annotations_len: usize,
    pub(crate) thinking_expanded_len: usize,
}

#[derive(Clone)]
pub(crate) struct CompletedMsgCache {
    pub(crate) key: CompletedMsgCacheKey,
    pub(crate) lines: Vec<RenderedLineItem>,
}

/// Memoizes the markdown render of the live streaming text (C1). During
/// streaming the frame loop redraws unconditionally (~20×/s) but `streaming_text`
/// only changes when a new Content chunk arrives, so most frames redraw with an
/// identical buffer. Without this we re-run syntect over the whole growing
/// buffer every frame — the `append_live_content` CPU hog called out in the
/// render-pipeline audit. Validity is checked by full content equality (cheap
/// next to syntect) so a flush→new-segment of the same length can't collide.
#[derive(Clone)]
pub(crate) struct StreamingTextCache {
    pub(crate) width: u16,
    pub(crate) text: String,
    pub(crate) lines: Vec<Line<'static>>,
}

thread_local! {
    pub(crate) static MESSAGE_LINES_CACHE: RefCell<Option<MessageLinesCache>> = const { RefCell::new(None) };
    /// Stores rendered lines for committed messages only; valid even during streaming.
    pub(crate) static COMPLETED_MSG_CACHE: RefCell<Option<CompletedMsgCache>> = const { RefCell::new(None) };
    pub(crate) static STREAMING_TEXT_CACHE: RefCell<Option<StreamingTextCache>> = const { RefCell::new(None) };
}

