// Vendored from jcode (crates/operant-tui/src/tui/markdown.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole file
// (103 lines). Deltas: `operant_tui_markdown::` re-rooted to
// `crate::tui::operant_markdown::`; `crate::config::` re-rooted to
// `crate::tui::operant_app::config_shim::`; `crate::logging::` re-rooted to
// `crate::tui::operant_app::logging::`; `crate::process_memory::` re-rooted to
// `crate::tui::operant_app::process_memory::`.
// [port-unresolved] the re-export list still names encode_handterm_latex_apc /
// handterm_native_latex_for_hash / set_latex_log_hook-adjacent items that
// operant_markdown excised with markdown_latex_image.rs (see
// operant_markdown/mod.rs:90-:93): every `pub use` that does not resolve in
// operant_markdown is commented out with a [port-unresolved] marker; the two
// integrator options are re-adding the latex module or excising callers.
#[cfg(test)]
pub use crate::tui::operant_markdown::thread_render_count;
pub use crate::tui::operant_markdown::{
    center_code_blocks, highlight_line, line_is_mermaid_pending_placeholder,
    recenter_structured_blocks_for_display, render_markdown, render_markdown_with_width,
    set_center_code_blocks, wrap_line, wrap_lines,
};
// [port-unresolved] encode_handterm_latex_apc: not exported by operant's operant_markdown
// [port-unresolved] handterm_native_latex_for_hash: not exported by operant's operant_markdown

fn to_markdown_diagram_mode(
    mode: crate::tui::operant_app::config_shim::DiagramDisplayMode,
) -> crate::tui::operant_markdown::DiagramDisplayMode {
    match mode {
        crate::tui::operant_app::config_shim::DiagramDisplayMode::None => {
            crate::tui::operant_markdown::DiagramDisplayMode::None
        }
        crate::tui::operant_app::config_shim::DiagramDisplayMode::Margin => {
            crate::tui::operant_markdown::DiagramDisplayMode::Margin
        }
        crate::tui::operant_app::config_shim::DiagramDisplayMode::Pinned => {
            crate::tui::operant_markdown::DiagramDisplayMode::Pinned
        }
    }
}

/// Run `f` with the diagram display mode pinned on the current thread only.
/// Unlike `set_diagram_mode_override`, this never mutates process-global
/// state, so concurrent renders (and parallel tests) are unaffected.
pub fn with_diagram_mode_scope<T>(
    mode: crate::tui::operant_app::config_shim::DiagramDisplayMode,
    f: impl FnOnce() -> T,
) -> T {
    crate::tui::operant_markdown::with_diagram_mode_scope(to_markdown_diagram_mode(mode), f)
}

pub fn with_deferred_mermaid_render_context<T>(f: impl FnOnce() -> T) -> T {
    crate::tui::operant_markdown::with_deferred_mermaid_render_context(f)
}

pub fn encode_handterm_latex_apc(source: &str) -> Option<String> {
    if source.bytes().any(|byte| matches!(byte, 0x07 | 0x1b)) {
        return None;
    }
    Some(format!("\x1b_L;{source}\x1b\\"))
}
