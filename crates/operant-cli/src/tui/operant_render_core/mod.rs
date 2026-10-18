// Vendored from jcode (crates/operant-render-core/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 as a module:
// lib.rs became mod.rs; internal `crate::` self-references re-rooted to
// crate::tui::operant_render_core. The crate exists here because the W2
// markdown port (operant_markdown) is a live consumer of its math/model/
// reasoning surface - unlike the stale vendored copy deleted at iter-581,
// which had zero consumers.

//! # operant-render-core
//!
//! Backend-neutral document/render model shared by jcode's front-ends (the
//! ratatui TUI and the desktop GPU UI).
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
//! for the TUI, glyph runs for the desktop), and supplies a
//! [`wrap::WidthMeasure`] for its width units.
//!
//! This model is extracted from the *working* TUI markdown renderer
//! (`operant-tui-markdown`); that renderer remains authoritative until this core
//! reaches parity, after which front-ends migrate onto it.

pub mod markdown;
pub mod math;
pub mod model;
pub mod preprocess;
pub mod reasoning;
pub mod wrap;

#[cfg(test)]
pub use markdown::parse_markdown;
pub use math::{render_display_latex, render_inline_latex};
pub use model::Alignment;
#[cfg(test)]
pub use model::{BlockKind, Document, FillRole, StyleRole, StyledLine, StyledSpan};
pub use preprocess::normalize_latex_math;
pub use reasoning::{
    REASONING_SENTINEL, reasoning_line_markup, reasoning_partial_markup,
    reasoning_summary_line_markup,
};
#[cfg(test)]
pub use wrap::{ColumnWidth, WidthMeasure, wrap_line};

#[cfg(test)]
mod tests;
