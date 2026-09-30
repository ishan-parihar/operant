# Vendored: jcode-tui-workspace

Workspace-map widget (model + ratatui renderer + color-capability detection) vendored
from jcode as an internal operant module. Upstream is a standalone Cargo crate; here it
is a directory module under `crates/operant-cli/src/tui/vendor/workspace/`, wired in by
the integrator via `tui::vendor::mod.rs` (not owned by this vendoring pass).

## Upstream

| Field | Value |
|---|---|
| Repository | https://github.com/1jehuang/jcode |
| Path | `crates/jcode-tui-workspace/` |
| Commit | `0a9dc7805db1d264bdaa96b6b8cea83c2c915a80` |
| Files | `Cargo.toml`, `src/lib.rs`, `src/color_support.rs`, `src/workspace_map.rs`, `src/workspace_map_widget.rs` (1,232 LOC) |
| Licence | MIT, Copyright (c) 2025 Jeremy Huang |
| Upstream deps | `ratatui = "0.30"` only |

## Files here

| File | Upstream counterpart | LOC (after header) |
|---|---|---|
| `mod.rs` | `src/lib.rs` | 7 |
| `color_support.rs` | `src/color_support.rs` | 486 |
| `workspace_map.rs` | `src/workspace_map.rs` | 414 |
| `workspace_map_widget.rs` | `src/workspace_map_widget.rs` | 339 |
| **total** | | **1,246** |

`Cargo.toml` is intentionally **not** reproduced: the crate's single dependency
(`ratatui = "0.30"`) is already a direct dependency of `operant-cli` at `0.30.2`, so the
module resolves against the binary's existing dependency.

## Adaptations

1. **Crate root → module root.** `src/lib.rs` became `mod.rs`; its three `pub mod`
   declarations are unchanged. No code referenced the upstream crate root by name, so
   no path rewrites were needed beyond the item below.
2. **Intra-crate imports `crate::` → `super::`.** `workspace_map_widget.rs` used
   `use crate::color_support::rgb;` and `use crate::workspace_map::{...}` (plus the same
   in its test module, two levels up). Inside `tui::vendor::workspace` those must be
   `super::…` / `super::super::…`, since `crate::` would resolve against `operant-cli`'s
   root. 3 import lines rewritten; no logic touched.
3. **Environment override renamed.** `color_support.rs` read
   `JCODE_GLYPH_SAFE_MODE=on|off` to force the fragile-glyph-cache colour downgrade on or
   off. Renamed to **`OPERANT_GLYPH_SAFE_MODE`** (1 production read + 5 test call sites)
   so no jcode-branded knob ships inside operant. Behaviour is otherwise identical.
4. **Doc-comment de-jcode-ing.** `color_support.rs`'s `fragile_glyph_cache_terminal` doc
   referenced the upstream sibling crate `jcode_tui_style::color` (twice) and an upstream
   issue number (`#330`, also in an inline comment). Rewritten to describe the behaviour
   without naming a crate or issue that do not exist in operant. No filesystem path or
   upstream repo/crate identifier survives in any vendored file.
5. **Licence header.** Every `.rs` file carries the two-line MIT attribution header
   required by the vendoring brief.

## Verified against operant-cli

- `ratatui` resolves at `0.30.2`, matching upstream's `0.30` requirement.
- Every ratatui API used by this crate exists in `ratatui 0.30.2`
  (`ratatui-core 0.1.2`): `Buffer` (`empty`, `content`, `Index`/`IndexMut` for
  `(u16, u16)` via `impl From<(u16, u16)> for Position`), `Cell`
   (`symbol`, `set_symbol`, `set_style`, `style`, `reset`), `Rect`
   (`left`, `right`, `top`, `bottom`, `new`), `Style` (`fg`, `add_modifier`),
   `Modifier::BOLD`, `Color::{Rgb, Indexed}`.
- No `.unwrap()` / `.expect()` outside `#[cfg(test)]` code. The one production-ish
  `lock().unwrap_or_else(|p| p.into_inner())` in `fragile_glyph_tests` is test-only.

## Tests ported

- `color_support.rs` → `mod tests` (11 tests: xterm-256 cube/grayscale quantisation,
  hue-tie breaking, near-neutral and light-blue regression cases).
- `color_support.rs` → `mod fragile_glyph_tests` (3 tests, one `#[cfg(target_os =
  "macos")]`, covering the `OPERANT_GLYPH_SAFE_MODE` override).
- `workspace_map.rs` → `mod tests` (5 tests: insertion order, refocus-relative insert,
  per-workspace focus memory, visible-row selection, visual-state preservation).
- `workspace_map_widget.rs` → `mod tests` (5 tests: placement centring/ordering,
  focused-tile glyph, completed-tile colour, spinner frames across ticks, narrow-area
  clipping).

24 tests are declared; **23 pass on Linux**, with `detects_vscode_and_apple_terminal` gated
`#[cfg(target_os = "macos")]` and therefore not compiled here. All 23 pass under the
module-path filter `tui::vendor::workspace`.

One adaptation to note in `workspace_map_widget.rs`'s test module: upstream wrote
`use crate::workspace_map::{…}`, which from inside `workspace_map_widget::tests` is two
levels up, so it is `super::super::workspace_map::{…}` here — not `super::`, as the
module-top import is. Caught by compiling the test target; `cargo check` alone does not
see `#[cfg(test)]` code.