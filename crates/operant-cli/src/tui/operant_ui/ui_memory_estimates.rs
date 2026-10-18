// Vendored from jcode (crates/operant-tui/src/tui/ui_memory_estimates.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; super:: parent
// refs stay as-is. See operant_ui/mod.rs for scope.

use ratatui::text::{Line, Span};
use std::sync::Arc;

#[cfg(test)]
use super::TEST_VISIBLE_COPY_TARGETS;
#[cfg(not(test))]
use super::visible_copy_targets_state;
use super::{
    CopyTarget, CopyTargetKind, EditToolRange, ImageRegion, PreparedChatFrame, PreparedMessages,
    PreparedSection, VisibleCopyTarget, WrappedLineMap, body_cache, full_prep_cache, pinned_ui,
};

fn estimate_lines_bytes(lines: &[Line<'static>]) -> usize {
    lines
        .iter()
        .map(|line| {
            std::mem::size_of::<Line<'static>>()
                + line.spans.capacity() * std::mem::size_of::<Span<'static>>()
                + line
                    .spans
                    .iter()
                    .map(|span| span.content.len())
                    .sum::<usize>()
        })
        .sum()
}

fn estimate_arc_string_vec_bytes(values: &Arc<Vec<String>>) -> usize {
    std::mem::size_of::<Vec<String>>()
        + values.capacity() * std::mem::size_of::<String>()
        + values.iter().map(|value| value.capacity()).sum::<usize>()
}

fn estimate_arc_usize_vec_bytes(values: &Arc<Vec<usize>>) -> usize {
    std::mem::size_of::<Vec<usize>>() + values.capacity() * std::mem::size_of::<usize>()
}

fn estimate_arc_wrapped_line_map_bytes(values: &Arc<Vec<WrappedLineMap>>) -> usize {
    std::mem::size_of::<Vec<WrappedLineMap>>()
        + values.capacity() * std::mem::size_of::<WrappedLineMap>()
}

fn estimate_copy_target_kind_bytes(kind: &CopyTargetKind) -> usize {
    match kind {
        CopyTargetKind::CodeBlock { language } => {
            language.as_ref().map(|value| value.capacity()).unwrap_or(0)
        }
        CopyTargetKind::Blockquote => 0,
        CopyTargetKind::Math { .. } => 0,
        CopyTargetKind::Error => 0,
        CopyTargetKind::ToolOutput => 0,
    }
}

fn estimate_copy_targets_bytes(values: &Vec<CopyTarget>) -> usize {
    values
        .iter()
        .map(|target| estimate_copy_target_kind_bytes(&target.kind) + target.content.capacity())
        .sum::<usize>()
        + values.capacity() * std::mem::size_of::<CopyTarget>()
}

fn estimate_edit_tool_ranges_bytes(values: &Vec<EditToolRange>) -> usize {
    values
        .iter()
        .map(|range| range.file_path.capacity())
        .sum::<usize>()
        + values.capacity() * std::mem::size_of::<EditToolRange>()
}

fn estimate_string_vec_bytes(values: &Vec<String>) -> usize {
    values.iter().map(|value| value.capacity()).sum::<usize>()
        + values.capacity() * std::mem::size_of::<String>()
}

fn estimate_image_regions_bytes(values: &Vec<ImageRegion>) -> usize {
    values.capacity() * std::mem::size_of::<ImageRegion>()
}

fn estimate_usize_vec_bytes(values: &Vec<usize>) -> usize {
    values.capacity() * std::mem::size_of::<usize>()
}

pub(super) fn estimate_prepared_messages_bytes(prepared: &PreparedMessages) -> usize {
    estimate_lines_bytes(&prepared.wrapped_lines)
        + estimate_arc_string_vec_bytes(&prepared.wrapped_plain_lines)
        + estimate_arc_usize_vec_bytes(&prepared.wrapped_copy_offsets)
        + estimate_arc_string_vec_bytes(&prepared.raw_plain_lines)
        + estimate_arc_wrapped_line_map_bytes(&prepared.wrapped_line_map)
        + estimate_usize_vec_bytes(&prepared.wrapped_user_indices)
        + estimate_usize_vec_bytes(&prepared.wrapped_user_prompt_starts)
        + estimate_usize_vec_bytes(&prepared.wrapped_user_prompt_ends)
        + estimate_string_vec_bytes(&prepared.user_prompt_texts)
        + estimate_image_regions_bytes(&prepared.image_regions)
        + estimate_edit_tool_ranges_bytes(&prepared.edit_tool_ranges)
        + estimate_copy_targets_bytes(&prepared.copy_targets)
}

pub(super) fn estimate_prepared_chat_frame_bytes(prepared: &PreparedChatFrame) -> usize {
    // Include each section's prepared content: the wrapped lines dominate a
    // frame's real memory footprint. Without this, multi-megabyte transcripts
    // estimate as a few hundred bytes, the oversized-entry path is dead code,
    // and the FULL_PREP_CACHE_MAX_BYTES budget cannot bound anything.
    prepared
        .sections
        .iter()
        .map(|section| estimate_prepared_messages_bytes(&section.prepared))
        .sum::<usize>()
        + prepared.sections.capacity() * std::mem::size_of::<PreparedSection>()
        + estimate_usize_vec_bytes(&prepared.wrapped_user_indices)
        + estimate_usize_vec_bytes(&prepared.wrapped_user_prompt_starts)
        + estimate_usize_vec_bytes(&prepared.wrapped_user_prompt_ends)
        + estimate_string_vec_bytes(&prepared.user_prompt_texts)
        + estimate_image_regions_bytes(&prepared.image_regions)
        + estimate_edit_tool_ranges_bytes(&prepared.edit_tool_ranges)
        + estimate_copy_targets_bytes(&prepared.copy_targets)
}
