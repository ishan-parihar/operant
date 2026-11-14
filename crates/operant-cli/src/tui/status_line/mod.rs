//! Status-line bar vocabulary (plan 2026-11-14 U7).
//!
//! Operant's status row is assembled imperatively in
//! `operant_ui::ui_input::draw_status`: one `Line` built from ad-hoc
//! `format!(" · {label}")` spans. That works, but it means the separator is not
//! a thing that can be named or changed in one place, and a filled bar is not
//! expressible at all.
//!
//! This module is the vocabulary that assembly will be rewritten against:
//! - [`separators`] — the seven separator styles, transcribed from oh-my-pi
//!   (`packages/tui/src/status-line/separators.ts`), plus the cap glyphs.
//! - [`bar`] — the fill: one background colour behind the whole row, from
//!   `Role::StatusLineBg` (OMP's `statusLineBg`).

pub mod bar;
pub mod separators;
