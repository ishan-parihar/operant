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
//!
//! A segment model and a separator vocabulary were built here at U7/U8 and then
//! DELETED: they had no production caller, because converting `draw_status`'s 51
//! hand-written ` · ` joins is a rewiring that changes what the bar looks like at
//! every width. Shipping a vocabulary nothing speaks is worse than not having
//! one — the next reader cannot tell which half is real, and unexercised code
//! rots. If that rewiring is ever done, restore the two modules from the commit
//! that deleted them and wire them in the SAME change.

pub mod bar;
pub mod meter;
