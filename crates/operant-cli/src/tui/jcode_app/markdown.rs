// Vendored from jcode (crates/jcode-tui/src/tui/markdown.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; whole file
// (103 lines). Deltas: `jcode_tui_markdown::` re-rooted to
// `crate::tui::jcode_markdown::`; `crate::config::` re-rooted to
// `crate::tui::jcode_app::config_shim::`; `crate::logging::` re-rooted to
// `crate::tui::jcode_app::logging::`; `crate::process_memory::` re-rooted to
// `crate::tui::jcode_app::process_memory::`.
// [port-unresolved] the re-export list still names encode_handterm_latex_apc /
// handterm_native_latex_for_hash / set_latex_log_hook-adjacent items that
// jcode_markdown excised with markdown_latex_image.rs (see
// jcode_markdown/mod.rs:90-:93): every `pub use` that does not resolve in
// jcode_markdown is commented out with a [port-unresolved] marker; the two
// integrator options are re-adding the latex module or excising callers.
pub use crate::tui::jcode_markdown::{
    CopyTargetKind, IncrementalMarkdownRenderer, MERMAID_PENDING_PLACEHOLDER_TEXT, MarkdownDebugStats, MarkdownMemoryProfile, RawCopyTarget, center_code_blocks, debug_memory_profile, debug_stats, debug_stats_json, extract_copy_targets_from_rendered_lines, highlight_file_lines, highlight_line, line_is_mermaid_pending_placeholder, mermaid_rendering_enabled, progress_bar, progress_line, recenter_structured_blocks_for_display, render_markdown, render_markdown_lazy, render_markdown_with_width, render_table_with_width, reset_debug_stats, set_center_code_blocks, thread_render_count, with_center_code_blocks, with_mermaid_rendering_override, wrap_line, wrap_lines,
};
// [port-unresolved] encode_handterm_latex_apc: not exported by operant's jcode_markdown
// [port-unresolved] handterm_native_latex_for_hash: not exported by operant's jcode_markdown


fn to_markdown_diagram_mode(
    mode: crate::tui::jcode_app::config_shim::DiagramDisplayMode,
) -> crate::tui::jcode_markdown::DiagramDisplayMode {
    match mode {
        crate::tui::jcode_app::config_shim::DiagramDisplayMode::None => crate::tui::jcode_markdown::DiagramDisplayMode::None,
        crate::tui::jcode_app::config_shim::DiagramDisplayMode::Margin => crate::tui::jcode_markdown::DiagramDisplayMode::Margin,
        crate::tui::jcode_app::config_shim::DiagramDisplayMode::Pinned => crate::tui::jcode_markdown::DiagramDisplayMode::Pinned,
    }
}

fn from_markdown_diagram_mode(
    mode: crate::tui::jcode_markdown::DiagramDisplayMode,
) -> crate::tui::jcode_app::config_shim::DiagramDisplayMode {
    match mode {
        crate::tui::jcode_markdown::DiagramDisplayMode::None => crate::tui::jcode_app::config_shim::DiagramDisplayMode::None,
        crate::tui::jcode_markdown::DiagramDisplayMode::Margin => crate::tui::jcode_app::config_shim::DiagramDisplayMode::Margin,
        crate::tui::jcode_markdown::DiagramDisplayMode::Pinned => crate::tui::jcode_app::config_shim::DiagramDisplayMode::Pinned,
    }
}

fn to_markdown_spacing_mode(
    mode: crate::tui::jcode_app::config_shim::MarkdownSpacingMode,
) -> crate::tui::jcode_markdown::MarkdownSpacingMode {
    match mode {
        crate::tui::jcode_app::config_shim::MarkdownSpacingMode::Compact => {
            crate::tui::jcode_markdown::MarkdownSpacingMode::Compact
        }
        crate::tui::jcode_app::config_shim::MarkdownSpacingMode::Document => {
            crate::tui::jcode_markdown::MarkdownSpacingMode::Document
        }
    }
}

fn to_markdown_latex_mode(
    mode: crate::tui::jcode_app::config_shim::LatexRenderingMode,
) -> crate::tui::jcode_markdown::LatexRenderingMode {
    match mode {
        crate::tui::jcode_app::config_shim::LatexRenderingMode::None => crate::tui::jcode_markdown::LatexRenderingMode::None,
        crate::tui::jcode_app::config_shim::LatexRenderingMode::Unicode => {
            crate::tui::jcode_markdown::LatexRenderingMode::Unicode
        }
        crate::tui::jcode_app::config_shim::LatexRenderingMode::Image => crate::tui::jcode_markdown::LatexRenderingMode::Image,
    }
}

pub fn install_jcode_markdown_hooks() {
    crate::tui::jcode_markdown::set_latex_log_hook(|error| {
        crate::tui::jcode_app::logging::warn(&format!(
            "LaTeX image rendering fell back to Unicode: {error}"
        ));
    });
    crate::tui::jcode_markdown::set_config_snapshot_hook(|| {
        let cfg = crate::tui::jcode_app::config_shim::config();
        crate::tui::jcode_markdown::MarkdownConfigSnapshot {
            diagram_mode: to_markdown_diagram_mode(cfg.display.diagram_mode),
            markdown_spacing: to_markdown_spacing_mode(cfg.display.markdown_spacing),
            mermaid_enabled: cfg.features.mermaid,
            latex_rendering: to_markdown_latex_mode(cfg.display.latex_rendering),
        }
    });
    crate::tui::jcode_markdown::set_memory_snapshot_hook(|| {
        let snapshot = crate::tui::jcode_app::process_memory::snapshot_with_source("client:markdown:memory");
        crate::tui::jcode_markdown::ProcessMemorySnapshot {
            rss_bytes: snapshot.rss_bytes,
            peak_rss_bytes: snapshot.peak_rss_bytes,
            virtual_bytes: snapshot.virtual_bytes,
        }
    });
}

pub fn set_diagram_mode_override(mode: Option<crate::tui::jcode_app::config_shim::DiagramDisplayMode>) {
    crate::tui::jcode_markdown::set_diagram_mode_override(mode.map(to_markdown_diagram_mode));
}

pub fn get_diagram_mode_override() -> Option<crate::tui::jcode_app::config_shim::DiagramDisplayMode> {
    crate::tui::jcode_markdown::get_diagram_mode_override().map(from_markdown_diagram_mode)
}

/// Run `f` with the diagram display mode pinned on the current thread only.
/// Unlike `set_diagram_mode_override`, this never mutates process-global
/// state, so concurrent renders (and parallel tests) are unaffected.
pub fn with_diagram_mode_scope<T>(
    mode: crate::tui::jcode_app::config_shim::DiagramDisplayMode,
    f: impl FnOnce() -> T,
) -> T {
    crate::tui::jcode_markdown::with_diagram_mode_scope(to_markdown_diagram_mode(mode), f)
}

pub fn with_deferred_mermaid_render_context<T>(f: impl FnOnce() -> T) -> T {
    crate::tui::jcode_markdown::with_deferred_mermaid_render_context(f)
}
