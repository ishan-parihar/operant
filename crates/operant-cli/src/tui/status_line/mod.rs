//! Status-line bar vocabulary (plan 2026-11-14 U7).
//!
//! Operant's status row is assembled imperatively in
//! `operant_ui::ui_input::draw_status`: one `Line` built from ad-hoc
//! `format!(" · {label}")` spans. That works, but it means the separator is not
//! a thing that can be named or changed in one place, and a filled bar is not
//! expressible at all.
//!
//! This module holds what that assembly actually uses today:
//! - [`bar`] — the fill: one background colour behind the whole row, from
//!   `Role::StatusLineBg` (OMP's `statusLineBg`).
//! - [`meter`] — the one `█`/`░` meter the five hand-written copies now share.
//! - [`separator`] — the one boundary drawn between two segments (U10), and the
//!   record of which ` · ` sites are boundaries rather than prose.
//! - [`style`] — which glyph draws that boundary (U11), from `[tui] bar_style`.
//!
//! A segment model and an earlier separator vocabulary were built here at U7/U8
//! and then DELETED: they had no production caller, because converting `draw_status`'s
//! 51 hand-written ` · ` joins is a rewiring that changes what the bar looks like at
//! every width. Shipping a vocabulary nothing speaks is worse than not having
//! one — the next reader cannot tell which half is real, and unexercised code
//! rots. `separator` is the first piece of that vocabulary with a live caller;
//! the rest returns only when the bar is actually made of segments.

pub mod bar;
pub mod meter;
pub mod separator;
pub mod style;
