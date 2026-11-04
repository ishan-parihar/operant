// Vendored from jcode (crates/operant-tui-messages/src/cache.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; only the two
// imports were re-rooted. See operant_model/mod.rs for scope.

use super::DisplayMessage;
use super::vendor_types::DiffDisplayMode;
use crate::tui::operant_app::config_shim::DiagramDisplayMode;
use ratatui::layout::Alignment;
use ratatui::text::{Line, Span};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MessageCacheKey {
    width: u16,
    diff_mode: DiffDisplayMode,
    message_hash: u64,
    content_len: usize,
    // Per-row expand/collapse. A row's diff renders its full body when the row
    // itself is expanded, and the diff is parsed from the tool call's INPUT plus
    // the message content -- neither of which changes when only the row's
    // expansion toggles. Without this key the cache hands back the collapsed
    // body for an expanded row, silently ignoring the click. (Tool-OUTPUT
    // expansion gets away without it only because adapter.rs swaps the full text
    // into msg.content, moving content_len and message_hash; a diff has no such
    // accident.)
    row_expanded: bool,
    diagram_mode: DiagramDisplayMode,
    centered: bool,
    mermaid_epoch: u64,
    mermaid_aspect_bucket: Option<u16>,
    show_agentgrep_output: bool,
    show_bash_output: bool,
    tool_call_details: bool,
}

#[derive(Default)]
struct MessageCacheState {
    entries: HashMap<MessageCacheKey, Arc<Vec<Line<'static>>>>,
    order: VecDeque<MessageCacheKey>,
}

impl MessageCacheState {
    fn get(&self, key: &MessageCacheKey) -> Option<Vec<Line<'static>>> {
        self.entries.get(key).map(|arc| arc.as_ref().clone())
    }

    fn insert(&mut self, key: MessageCacheKey, lines: Vec<Line<'static>>) {
        let arc = Arc::new(lines);
        if let std::collections::hash_map::Entry::Occupied(mut entry) =
            self.entries.entry(key.clone())
        {
            entry.insert(arc);
            return;
        }

        self.entries.insert(key.clone(), arc);
        self.order.push_back(key);

        while self.order.len() > MESSAGE_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

static MESSAGE_CACHE: OnceLock<Mutex<MessageCacheState>> = OnceLock::new();

fn message_cache() -> &'static Mutex<MessageCacheState> {
    MESSAGE_CACHE.get_or_init(|| Mutex::new(MessageCacheState::default()))
}

const MESSAGE_CACHE_LIMIT: usize = 2048;

/// Runtime-sensitive inputs that affect message rendering but are not intrinsic to a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageCacheContext {
    pub diagram_mode: DiagramDisplayMode,
    pub centered: bool,
    pub mermaid_epoch: u64,
    pub mermaid_aspect_bucket: Option<u16>,
    pub show_agentgrep_output: bool,
    pub show_bash_output: bool,
    pub tool_call_details: bool,
}

pub fn left_pad_lines_for_centered_mode(lines: &mut [Line<'static>], width: u16) {
    let max_line_width = lines.iter().map(Line::width).max().unwrap_or(0);
    let pad = (width as usize).saturating_sub(max_line_width) / 2;
    if pad == 0 {
        return;
    }

    let pad_str = " ".repeat(pad);
    for line in lines {
        line.spans.insert(0, Span::raw(pad_str.clone()));
        line.alignment = Some(Alignment::Left);
    }
}

pub fn centered_wrap_width(width: u16, centered: bool, centered_max_width: usize) -> usize {
    let width = width as usize;
    if centered {
        width.min(centered_max_width).max(1)
    } else {
        width.max(1)
    }
}

pub fn get_cached_message_lines<F>(
    msg: &DisplayMessage,
    width: u16,
    diff_mode: DiffDisplayMode,
    row_expanded: bool,
    context: MessageCacheContext,
    render: F,
) -> Vec<Line<'static>>
where
    F: FnOnce(&DisplayMessage, u16, DiffDisplayMode, bool) -> Vec<Line<'static>>,
{
    if cfg!(test) {
        return render(msg, width, diff_mode, row_expanded);
    }

    let mut cache = match message_cache().lock() {
        Ok(c) => c,
        Err(poisoned) => poisoned.into_inner(),
    };
    let key = message_cache_key(msg, width, diff_mode, row_expanded, context);
    if let Some(lines) = cache.get(&key) {
        return lines;
    }

    let lines = render(msg, width, diff_mode, row_expanded);
    cache.insert(key, lines.clone());
    lines
}

/// The cache lookup key for one transcript row.
///
/// Split out of `get_cached_message_lines` for one reason: that function
/// short-circuits under `cfg!(test)` (line 122), so no in-process test can
/// exercise a cache HIT and therefore no test could observe a key that omits
/// a field. Factoring the derivation out lets a unit test call the SAME
/// construction site the production path calls, which is the only way the
/// key's completeness is provable at all.
fn message_cache_key(
    msg: &DisplayMessage,
    width: u16,
    diff_mode: DiffDisplayMode,
    row_expanded: bool,
    context: MessageCacheContext,
) -> MessageCacheKey {
    MessageCacheKey {
        width,
        diff_mode,
        message_hash: msg.stable_cache_hash(),
        content_len: msg.content.len(),
        row_expanded,
        diagram_mode: context.diagram_mode,
        centered: context.centered,
        mermaid_epoch: context.mermaid_epoch,
        mermaid_aspect_bucket: context.mermaid_aspect_bucket,
        show_agentgrep_output: context.show_agentgrep_output,
        show_bash_output: context.show_bash_output,
        tool_call_details: context.tool_call_details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_wrap_width_caps_centered_width() {
        assert_eq!(centered_wrap_width(120, true, 96), 96);
        assert_eq!(centered_wrap_width(80, true, 96), 80);
        assert_eq!(centered_wrap_width(120, false, 96), 120);
    }

    #[test]
    fn left_pad_lines_aligns_to_centered_block() {
        let mut lines = vec![Line::from("abc")];
        left_pad_lines_for_centered_mode(&mut lines, 9);
        assert_eq!(lines[0].to_string(), "   abc");
        assert_eq!(lines[0].alignment, Some(Alignment::Left));
    }

    /// RT-146: a row toggle must MISS the cache.
    ///
    /// This calls `message_cache_key` — the SAME construction site the
    /// production path uses — because `get_cached_message_lines`
    /// short-circuits under `cfg!(test)` and therefore no test can reach a
    /// cache HIT. An earlier draft built the struct literal inline instead
    /// and stayed GREEN when the construction site was mutated to pin
    /// `row_expanded: false`; it proved the field existed in the type, not
    /// that it was populated, which is the assurance this test exists for.
    ///
    /// It matters for diffs specifically. Tool-OUTPUT expansion can afford a
    /// content-derived key because adapter.rs swaps the retained full text
    /// into `msg.content`, moving `content_len` and `message_hash`. A DIFF
    /// is parsed from the tool call's INPUT plus that content — neither
    /// changes when only a row's FULLNESS toggles. Without `row_expanded`
    /// in the key the two states are byte-identical keys and the cache
    /// serves the collapsed body to an expanded row, silently swallowing
    /// the click.
    #[test]
    fn a_row_toggle_changes_the_cache_key() {
        // A tool row whose INPUT carries the diff and whose content is the
        // tool's output text. Both stay byte-identical across the toggle,
        // which is the whole reason the key needs the flag.
        fn msg() -> DisplayMessage {
            DisplayMessage {
                role: "tool".to_string(),
                content: "Edited demo.txt".to_string(),
                tool_calls: Vec::new(),
                duration_secs: None,
                title: Some("demo.txt".to_string()),
                tool_data: Some(crate::tui::operant_model::vendor_types::ToolCall {
                    id: "call_row".to_string(),
                    name: "edit".to_string(),
                    input: serde_json::json!({
                        "file_path": "demo.txt",
                        "old_string": "a\n",
                        "new_string": "b\n",
                    }),
                    intent: None,
                    thought_signature: None,
                }),
            }
        }
        let context = MessageCacheContext {
            diagram_mode: DiagramDisplayMode::default(),
            centered: false,
            mermaid_epoch: 0,
            mermaid_aspect_bucket: None,
            show_agentgrep_output: false,
            show_bash_output: false,
            tool_call_details: false,
        };
        let collapsed = message_cache_key(&msg(), 80, DiffDisplayMode::Inline, false, context);
        let expanded = message_cache_key(&msg(), 80, DiffDisplayMode::Inline, true, context);

        assert_ne!(
            collapsed, expanded,
            "a collapsed and an expanded row must be different cache entries, \
             or the cache returns the truncated diff for a row the operator \
             expanded"
        );

        // And the rest of the key is untouched: the same row in the same
        // expansion state must still collide, or the cache would grow one
        // entry per render and never hit.
        assert_eq!(
            expanded,
            message_cache_key(&msg(), 80, DiffDisplayMode::Inline, true, context),
            "the same expansion state must collide — the key must not carry \
             anything per-render"
        );
    }
}
