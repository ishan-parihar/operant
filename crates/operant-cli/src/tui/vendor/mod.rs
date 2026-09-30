//! Vendored TUI subsystems from jcode (MIT, Copyright (c) 2025 Jeremy Huang).
//!
//! These modules were copied from `parent-projects/jcode/crates/` so that operant's TUI can adopt
//! jcode's visual design system without adopting jcode's application core, which is welded to
//! ~289k LOC of `jcode-app-core` + `jcode-base` and could not be lifted.
//!
//! Per-crate provenance, adaptations, and licence detail live in each directory's `VENDOR.md`.
//!
//! | module | upstream crate | LOC | purpose |
//! |---|---|---|---|
//! | [`style`] | `jcode-tui-style` | ~2.7k | 22 semantic colour roles, frozen default palette, buffer-level substitution |
//! | [`render_core`] | `jcode-render-core` | ~4.6k | markdown prep/parse, wrapping, LaTeX math — ratatui-free |
//! | [`workspace`] | `jcode-tui-workspace` | ~1.2k | workspace-map ratatui widget (a capability operant lacked) |
//! | [`anim`] | `jcode-tui-anim` | ~1.1k | dependency-free 3D samplers for the idle animation |
//!
//! # Why modules and not workspace crates
//!
//! The repository root `Cargo.toml` carries uncommitted work from another agent, so adding
//! `[workspace] members` entries here would have meant staging a line inside someone else's diff.
//! Every crate `operant-cli` already depends on (`ratatui` 0.30.2, `serde`, `serde_json`, `chrono`,
//! `crossterm`, `anyhow`, `dirs`, `pulldown-cmark`, `unicode-width`) is present, so the crates
//! resolve as internal modules with no manifest change. Promoting any of them to a standalone
//! crate later is a mechanical move.
//!
//! # Integration status
//!
//! As of the vendoring pass these modules compile but are **not yet called by the render path**.
//! The next step wires [`style::theme_mode::adapt_buffer_for_display`] into
//! `render::render_app` as the single per-frame substitution choke point, which is what makes
//! `/theme` repaint every surface. Until that lands, `style`'s roles are inert and operant still
//! reads its own `theme_colors`.
//!
//! See `docs/PLAN-TUI-OVERHAUL.md` §1 (lift manifest) and §2 (what was deliberately not lifted).

// Upstream these are four independent *library* crates, so every `pub` item is part of a public API
// and never dead. Vendored as modules of a binary crate, the whole surface is unreachable until the
// render path calls it — which produced 129 `dead_code` and 6 `unused_imports` (all `pub use`
// re-exports in `render_core`) the moment the tree was wired in.
//
// Both lints have the same single root cause and the same single fix, so they are suppressed once
// here instead of being allowlisted individually (135 allowlist entries for four files' worth of
// not-yet-wired code) or annotated in seven places.
//
// REMOVE OR NARROW THIS once `style` is wired into `render::render_app` — the `unused_imports`
// half can go as soon as `render_core` has a consumer, and the `dead_code` half as each surface
// adopts the vendored widgets. Tracked in docs/PLAN-TUI-OVERHAUL.md §3, tasks 1.6 and 2.5.
#![allow(dead_code, unused_imports)]

pub mod anim;
pub mod render_core;
pub mod style;
pub mod workspace;
