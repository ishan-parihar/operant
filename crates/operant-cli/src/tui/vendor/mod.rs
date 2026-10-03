//! Vendored TUI subsystem from jcode (MIT, Copyright (c) 2025 Jeremy Huang).
//!
//! This module was copied from `parent-projects/jcode/crates/` so that operant's TUI can adopt
//! jcode's visual design system without adopting jcode's application core, which is welded to
//! ~289k LOC of `jcode-app-core` + `jcode-base` and could not be lifted.
//!
//! Per-crate provenance, adaptations, and licence detail live in `style/VENDOR.md`.
//!
//! | module | upstream crate | LOC | purpose |
//! |---|---|---|---|
//! | [`style`] | `jcode-tui-style` | ~2.7k | 22 semantic colour roles, frozen default palette, buffer-level substitution |
//!
//! # Why modules and not workspace crates
//!
//! The repository root `Cargo.toml` carries uncommitted work from another agent, so adding a
//! `[workspace] members` entry here would have meant staging a line inside someone else's diff.
//! Every crate `operant-cli` already depends on (`ratatui` 0.30.2, `serde`, `serde_json`,
//! `chrono`, `crossterm`, `anyhow`, `dirs`, `pulldown-cmark`, `unicode-width`) is present, so the
//! crate resolves as an internal module with no manifest change. Promoting it to a standalone
//! crate later is a mechanical move.
//!
//! # Integration status
//!
//! As of the vendoring pass this module compiles but is **not yet called by the render path**.
//! The next step wires [`style::theme_mode::adapt_buffer_for_display`] into
//! `render::render_app` as the single per-frame substitution choke point, which is what makes
//! `/theme` repaint every surface. Until that lands, `style`'s roles are inert and operant still
//! reads its own `theme_colors`.

// Upstream this is an independent *library* crate, so every `pub` item is part of a public API
// and never dead. Vendored as a module of a binary crate, the public surface is unreachable
// until the render path calls it — which produced `dead_code` warnings the moment the tree was
// wired in. (The vendoring pass originally saw 129 `dead_code` + 6 `unused_imports`; the Wave 0
// purge removed the three zero-consumer modules that owned the great majority of those —
// including all 6 re-export `unused_imports`.)
//
// The lint has a single root cause and a single fix, so it is suppressed once here instead of
// being allowlisted item-by-item or annotated at every definition.
//
// REMOVE THIS once `style` is wired into `render::render_app` and every role has a consumer —
// at that point no `pub` item in `style` should be dead, so the suppressor and this comment go
// away together. Tracked in docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md §2 and
// docs/PLAN-TUI-OVERHAUL.md §3, tasks 1.6 and 2.5.
#![allow(dead_code, unused_imports)]

pub mod style;
