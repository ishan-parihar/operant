//! Markdown table rendering.
//!
//! Block structure (lists, headings, block quotes, code fences, tables and
//! footnotes) is parsed by `pulldown_cmark` in `markdown.rs`, which detects a
//! table and hands the extracted cells to `render_table` here. This module
//! owns only the box-drawing presentation: column sizing, per-column
//! alignment and truncation to the wrap width.
