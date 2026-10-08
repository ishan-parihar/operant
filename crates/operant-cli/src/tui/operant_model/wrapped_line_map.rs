// Vendored from jcode (crates/operant-tui-messages/src/wrapped_line_map.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// no changes. See operant_model/mod.rs for scope.

#[derive(Clone, Copy, Debug)]
pub struct WrappedLineMap {
    pub raw_line: usize,
    pub start_col: usize,
    pub end_col: usize,
}
