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
#[allow(unused_imports)] // re-export surface: the transcript renderers land at the cutover
pub use crate::tui::operant_markdown::{
    CopyTargetKind, IncrementalMarkdownRenderer, MERMAID_PENDING_PLACEHOLDER_TEXT,
    MarkdownDebugStats, MarkdownMemoryProfile, RawCopyTarget, center_code_blocks,
    debug_memory_profile, debug_stats, debug_stats_json, extract_copy_targets_from_rendered_lines,
    highlight_file_lines, highlight_line, line_is_mermaid_pending_placeholder,
    mermaid_rendering_enabled, progress_bar, progress_line, recenter_structured_blocks_for_display,
    render_markdown, render_markdown_lazy, render_markdown_with_width, render_table_with_width,
    reset_debug_stats, set_center_code_blocks, thread_render_count, with_center_code_blocks,
    with_mermaid_rendering_override, wrap_line, wrap_lines,
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

fn from_markdown_diagram_mode(
    mode: crate::tui::operant_markdown::DiagramDisplayMode,
) -> crate::tui::operant_app::config_shim::DiagramDisplayMode {
    match mode {
        crate::tui::operant_markdown::DiagramDisplayMode::None => {
            crate::tui::operant_app::config_shim::DiagramDisplayMode::None
        }
        crate::tui::operant_markdown::DiagramDisplayMode::Margin => {
            crate::tui::operant_app::config_shim::DiagramDisplayMode::Margin
        }
        crate::tui::operant_markdown::DiagramDisplayMode::Pinned => {
            crate::tui::operant_app::config_shim::DiagramDisplayMode::Pinned
        }
    }
}

fn to_markdown_spacing_mode(
    mode: crate::tui::operant_app::config_shim::MarkdownSpacingMode,
) -> crate::tui::operant_markdown::MarkdownSpacingMode {
    match mode {
        crate::tui::operant_app::config_shim::MarkdownSpacingMode::Compact => {
            crate::tui::operant_markdown::MarkdownSpacingMode::Compact
        }
        crate::tui::operant_app::config_shim::MarkdownSpacingMode::Document => {
            crate::tui::operant_markdown::MarkdownSpacingMode::Document
        }
    }
}

fn to_markdown_latex_mode(
    mode: crate::tui::operant_app::config_shim::LatexRenderingMode,
) -> crate::tui::operant_markdown::LatexRenderingMode {
    match mode {
        crate::tui::operant_app::config_shim::LatexRenderingMode::None => {
            crate::tui::operant_markdown::LatexRenderingMode::None
        }
        crate::tui::operant_app::config_shim::LatexRenderingMode::Unicode => {
            crate::tui::operant_markdown::LatexRenderingMode::Unicode
        }
        crate::tui::operant_app::config_shim::LatexRenderingMode::Image => {
            crate::tui::operant_markdown::LatexRenderingMode::Image
        }
    }
}

pub fn install_operant_markdown_hooks() {
    crate::tui::operant_markdown::set_latex_log_hook(|error| {
        crate::tui::operant_app::logging::warn(&format!(
            "LaTeX image rendering fell back to Unicode: {error}"
        ));
    });
    crate::tui::operant_markdown::set_config_snapshot_hook(|| {
        let cfg = crate::tui::operant_app::config_shim::config();
        crate::tui::operant_markdown::MarkdownConfigSnapshot {
            diagram_mode: to_markdown_diagram_mode(cfg.display.diagram_mode),
            markdown_spacing: to_markdown_spacing_mode(cfg.display.markdown_spacing),
            mermaid_enabled: cfg.features.mermaid,
            latex_rendering: to_markdown_latex_mode(cfg.display.latex_rendering),
        }
    });
    crate::tui::operant_markdown::set_memory_snapshot_hook(|| {
        let snapshot =
            crate::tui::operant_app::process_memory::snapshot_with_source("client:markdown:memory");
        crate::tui::operant_markdown::ProcessMemorySnapshot {
            rss_bytes: snapshot.rss_bytes,
            peak_rss_bytes: snapshot.peak_rss_bytes,
            virtual_bytes: snapshot.virtual_bytes,
        }
    });
}

pub fn set_diagram_mode_override(
    mode: Option<crate::tui::operant_app::config_shim::DiagramDisplayMode>,
) {
    crate::tui::operant_markdown::set_diagram_mode_override(mode.map(to_markdown_diagram_mode));
}

pub fn get_diagram_mode_override() -> Option<crate::tui::operant_app::config_shim::DiagramDisplayMode>
{
    crate::tui::operant_markdown::get_diagram_mode_override().map(from_markdown_diagram_mode)
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

// [port-decision] handterm native-latex surface: ui_viewport.rs (:23, :1015)
// calls encode_handterm_latex_apc + handterm_native_latex_for_hash through this
// module (upstream re-exports them from operant-tui-markdown/src/lib.rs:81-:84,
// defined in markdown_latex_image.rs:282/:289; that module is excised, so the
// signatures port verbatim here). HandtermNativeLatex (:44-:49) verbatim;
// encode_handterm_latex_apc (:282-:286) verbatim; handterm_native_latex_for_hash
// is degraded: the HANDTERM_NATIVE_LATEX cache it reads is only populated by the
// excised render path, so it honestly returns None. Re-activate at latex-image
// cutover.
#[derive(Clone, Debug)]
pub struct HandtermNativeLatex {
    pub source: String,
    pub display: bool,
    pub rows: u16,
    pub cols: u16,
}

pub fn encode_handterm_latex_apc(source: &str) -> Option<String> {
    if source.bytes().any(|byte| matches!(byte, 0x07 | 0x1b)) {
        return None;
    }
    Some(format!("\x1b_L;{source}\x1b\\"))
}

pub fn handterm_native_latex_for_hash(_hash: u64) -> Option<HandtermNativeLatex> {
    None
}
