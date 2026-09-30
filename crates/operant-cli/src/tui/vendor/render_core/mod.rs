// Vendored from jcode (crates/jcode-render-core), MIT License, Copyright (c) 2025 Jeremy Huang.
// Adapted for operant: crate root became a module root; the two upstream `tests/*.rs`
// integration targets became `#[cfg(test)]` submodules so their crate-root imports resolve;
// upstream-crate names in doc comments replaced with operant equivalents; three production
// `.unwrap()`/`.expect()` sites rewritten as non-panicking control flow for operant's clippy
// gate. No parsing, layout, or math logic changed.

//! Backend-neutral document/render model shared by operant's TUI front-ends.
//!
//! The pipeline is split at the seam where backends actually differ:
//!
//! ```text
//!   text ─▶ parse_markdown ─▶ Document (neutral blocks/spans) ─▶ wrap ─▶ adapter ─▶ backend draw
//! ```
//!
//! Everything up to and including wrapping is shared here. Each front-end owns
//! only a thin adapter: it resolves [`model::StyleRole`] to concrete colors and
//! turns [`model::StyledLine`]s into its own draw primitives (`ratatui::Line`
//! for the TUI) and supplies a [`wrap::WidthMeasure`] for its width units.
//!
//! Vendored verbatim from jcode's `jcode-render-core`, which upstream documents as
//! having no ratatui/GPU/glyph dependency. operant's existing markdown renderer
//! remains authoritative until a front-end is migrated onto this core.

pub mod markdown;
pub mod math;
pub mod model;
pub mod preprocess;
pub mod reasoning;
pub mod wrap;

pub use markdown::parse_markdown;
pub use math::{render_display_latex, render_inline_latex};
pub use model::{
    Alignment, Block, BlockKind, Document, FillRole, StyleRole, StyledLine, StyledSpan, TextAttrs,
};
pub use preprocess::{escape_currency_dollars, normalize_latex_math};
pub use reasoning::{
    REASONING_SENTINEL, reasoning_line_content, reasoning_line_markup, reasoning_partial_markup,
    reasoning_summary_line_markup,
};
pub use wrap::{ColumnWidth, WidthMeasure, wrap_line, wrap_lines};

#[cfg(test)]
mod tests;

// Upstream integration tests (`tests/latex_robustness.rs`,
// `tests/latex_streaming_regressions.rs`) had no home as submodules; they are
// cfg(test)-gated modules here so their `use super::{...}` imports resolve.
#[cfg(test)]
mod tests_latex_robustness;
#[cfg(test)]
mod tests_latex_streaming_regressions;
