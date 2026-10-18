// Vendored from jcode (crates/operant-tui-permissions/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-4 map. Decl needed in operant_app/mod.rs:
// `pub mod permissions;`.
// [port-decision] `operant_core::util::truncate_str` re-rooted to
// `crate::tui::operant_app::util::truncate_str` (the operant-core/src/util.rs port).
// [port-decision] `operant_tui_style::` re-rooted to `crate::tui::vendor::style::`
// (the established vendored home of operant-tui-style).
// `operant_base::safety` re-rooted to `super::safety` (operant_app/safety.rs,
// batch-4 sweep tail) — the file-backed permission-decision surface.
// unmapped/unported (see the batch-4 unresolved register).
