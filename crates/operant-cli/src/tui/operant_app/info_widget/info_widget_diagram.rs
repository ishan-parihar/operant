// Vendored from jcode (crates/operant-tui-mermaid/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] `DiagramInfo` (:132-:141) is the struct the ported renderers
// carry in their info-panel payloads; upstream reaches it through
// `pub use operant_tui_mermaid::DiagramInfo` at operant-tui/src/tui/info_widget.rs:551.
// The mermaid engine crate is not linked here (the W9 cut keeps only the
// cfg-fallback), so the data type is ported verbatim into its own leaf and
// re-exported at `info_widget::DiagramInfo` — the exact path the ported call
// sites already use.
#[derive(Debug, Clone)]
pub struct DiagramInfo {
    /// Hash for mermaid cache lookup
    pub hash: u64,
    /// Original PNG width
    pub width: u32,
    /// Original PNG height
    pub height: u32,
    /// Optional label/title
    pub label: Option<String>,
}
