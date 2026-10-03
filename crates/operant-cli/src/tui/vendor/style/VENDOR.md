# VENDOR.md — jcode TUI design system (`jcode-tui-style`)

## Provenance

| | |
|---|---|
| Upstream repository | https://github.com/1jehuang/jcode |
| Upstream path | `crates/jcode-tui-style/` |
| Upstream commit | `0a9dc7805db1d264bdaa96b6b8cea83c2c915a80` (2026-09-29) |
| Licence | MIT — Copyright (c) 2025 Jeremy Huang (repo root `LICENSE`) |
| Vendored on | 2026-09-30, for `docs/PLAN-TUI-OVERHAUL.md` §1/§2 |
| Reason | operant's TUI had ~300 hardcoded colors bypassing its palette. jcode's crate has 22 semantic roles with a single buffer-level substitution choke point (`adapt_buffer_for_display`). |

Every file in this directory carries a two-line header naming the upstream path,
the licence, the copyright holder, and the specific adaptations made to that
file.

## Files

| File | Upstream | Lines (ported) |
|---|---|---|
| `mod.rs` | `src/lib.rs` | 69 |
| `color.rs` | `src/color.rs` | 635 |
| `palette.rs` | `src/palette.rs` | 850 |
| `palette_literals.rs` | `src/palette_literals.rs` | 231 |
| `theme.rs` | `src/theme.rs` | 304 |
| `theme_mode.rs` | `src/theme_mode.rs` | 625 |

Total ported: 2,714 LOC (upstream, minus `harmony` and `examples`, plus headers
and doc rewrites). `palette_literals.rs` is an `include!` fragment, not a module,
so `rustfmt` cannot parse it standalone — same upstream, and it needs no
formatting.

## Deliberately left out

| Upstream | LOC | Why |
|---|---|---|
| `src/harmony.rs` + `src/harmony/{generate,graph,measured}.rs` | 2,357 | Explicitly out of scope per `docs/PLAN-TUI-OVERHAUL.md` §2(d) (Oklab palette scoring/generation). Nothing on operant's render path uses it. |
| `examples/light_bench.rs` | 237 | jcode-workspace `[[example]]` machinery; benchmarks the harmony generator, which is not vendored. |
| `Cargo.toml` | 21 | jcode crate manifest. The code is now an internal module of `operant-cli`, which already depends on `ratatui` (0.30 → locked 0.30.2, same minor as jcode) and `tracing`. |
| `src/tui/theme_detect.rs` (379 LOC) — **not part of `jcode-tui-style`** | — | See "Note on theme_detect" below. |

## Itemised adaptations

### 1. `mod.rs` (was `lib.rs`)

- **Renamed `lib.rs` → `mod.rs`.** Upstream this was a standalone crate root;
  vendored, it is a directory module, so a plain `pub mod style;` in
  `tui/vendor/mod.rs` must resolve to `style/mod.rs`. *(If you are writing
  `tui/vendor/mod.rs`: use `pub mod style;` — do not use a `#[path]` attribute.)*
- **Dropped `pub mod harmony;` and the `harmony` re-export line** (`Criterion`,
  `HarmonyReport`, `Oklab`, `analyze_harmony`, `analyze_active`) — out of scope.
- **Replaced `jcode_logging::warn(&format!(...))` with `tracing::warn!(%error, ...)`**
  in `restore_terminal_quietly`. `jcode-logging` pulls in jcode-core +
  jcode-storage + tokio, none of which exist in operant. `tracing::warn!` is
  operant-cli's established warning path (`gateway_runner.rs` and others) and is
  strictly safer than the `eprintln!` fallback the brief allowed: operant-cli's
  subscriber writes to a log file, `io::sink`, or stderr depending on
  `[logging]`, and is a no-op when no subscriber is installed — so it cannot
  panic on the dead terminal this function exists to tolerate.
- **Added `#![allow(dead_code)]` plus `#[allow(unused_imports)]` on the three
  `pub use` re-exports.** Vendoring a library crate's public surface into a
  *private* module of a binary makes rustc treat every not-yet-migrated caller
  as unreachable: measured at **101 warnings (97 `dead_code`, 4
  `unused_imports`)** in a private `mod tui;` binary. The gate script fails on
  any new warning, so this is load-bearing, not cosmetic. Scoped to this module
  only — real unused imports elsewhere still warn.

### 2. `color.rs`

- Rewrote the doc comment that cited `jcode_app_core::perf` and issue #330 to
  name "the app layer's terminal-detection policy" instead, and "the continuous
  color animations jcode emits" → "continuous per-cell color animations".
- Rewrote the second `jcode_app_core::perf::detect_terminal` reference to
  "Mirror of the app layer's terminal detection … (kept local to avoid a
  dependency from the style module)".
- `pin_truecolor_for_tests` doc no longer mentions harmony scoring or
  `role_for_rendered` (both dropped).
- **Code change (Wave 0 of docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md,
  item 3): the vendored depth detector is deleted.** `ColorCapability`,
  `color_capability`, `detect_color_capability`, `detect_raw_color_capability`,
  `fragile_glyph_cache_terminal`, `has_truecolor`, `pin_truecolor_for_tests`,
  the `CAPABILITY` / `CAPABILITY_OVERRIDE` statics, the `JCODE_GLYPH_SAFE_MODE`
  env var (the upstream issue-#330 glyph-safe override is gone with it), and
  the xterm-256 quantization branch of `rgb()` are all removed. `rgb()` now
  always returns `Color::Rgb`: one palette, no depth negotiation. The process
  has exactly one depth answer — `crate::tui::color_depth::detect` — and the
  quantization it implies runs once, on the operant theme palette inside
  `set_active_theme`, never inside this vendored layer. `indexed_to_rgb`
  survives because `theme_mode::color_rgb` maps indexed/named terminal
  colours through it.

### 3. `palette.rs`

- `crate::color::…` / `crate::theme_mode::…` → `super::…` (and `super::super::…`
  inside `#[cfg(test)]` submodules, which are two levels below the module root).
- **Dropped `role_for_rendered` and its `FAMILY_RADIUS` constant** (28 lines).
  They existed only to attribute *rendered* colors back to roles for the Oklab
  harmony graph — a measurement helper, explicitly documented upstream as "not on
  the render path and never applies an override". With `harmony` out of scope
  there is no consumer and no `Oklab` to measure with. The only caller upstream
  was jcode-tui's `ui_tests/palette_topology.rs`.
- **Retargeted `generating_and_scoring_do_not_disturb_the_default` →
  `installing_and_resetting_a_palette_do_not_disturb_the_default`.** The
  invariant it guards ("the shipped default palette is immutable") is kept; the
  harmony generator/scorer it exercised are gone, so it now asserts the same
  invariant against the palette's own process-global (install a configured
  palette, reset, compare to `Palette::default()`), under `STYLE_TEST_LOCK`.
- Module doc: the `~/.jcode/config.toml` + `[display.colors]` example was
  replaced with prose pointing at operant's `~/.operant/operant.toml` and at
  `Palette::from_pairs`, without inventing a config table that does not exist
  yet. **This was the only jcode path string in the vendored code.**
- Doc nits: `jcode's historical hard-coded palette` → `the historical …`;
  `/colors` slash-command references → "the user's palette" / "role listings";
  `harmony criteria` → `contrast criteria`; the frozen-table doc's
  "the generator, the harmony scorer" rationale now names only the light-theme
  repair pass.
- **The 22 roles, every `default_rgb()`, and the redundant `HAND_TUNED` frozen
  table are byte-for-byte unchanged.**

### 4. `palette_literals.rs`

- Header only. The 227-entry literal list is upstream's sweep of jcode's own
  `rgb(...)` call sites; operant's literals get added when its surfaces are
  rebuilt (the upstream header already says to regenerate it).

### 5. `theme.rs`

- `crate::color` / `crate::palette` → `super::…` in the two `use` statements and
  in all 22 role accessors. Accessor bodies are otherwise unchanged.
- No doc-comment changes were needed (the file named no jcode crate).

### 6. `theme_mode.rs`

- `crate::…` → `super::…` throughout, including inside `mod tests`.
- Module doc: "jcode's palette" → "The TUI's palette"; the mode-detection
  sentence no longer names `JCODE_THEME` / `display.theme`.
- `ThemeMode::Dark` doc: "jcode's native palette" → "the native palette".
- **Added `#[expect(clippy::unwrap_used, reason = "super::color::rgb only ever
  returns Rgb or Indexed, both of which color_rgb maps")]` on
  `readable_light_foreground`.** The one production `.unwrap()` in the vendored
  code (line 166) would otherwise be a new gate violation; operant forbids
  `.unwrap()` in production outside justified `#[expect]` sites.
- The substitution algorithm, the 7:1 contrast target, `LIGHT_SURFACE`, the
  `include!("palette_literals.rs")` coverage sweep, and every test assertion
  are unchanged.

## Note on `theme_detect`

The brief said to repoint `theme_detect`'s OSC-11 cache path. **`theme_detect` is
not part of `jcode-tui-style`** — it lives in the consuming crate at
`crates/jcode-tui/src/tui/theme_detect.rs` (379 LOC), is declared in
`crates/jcode-tui/src/tui/mod.rs:68`, and its cache path goes through
`crate::storage::jcode_dir()`. It is not in the 11-file / 5,256-LOC source
inventory this port was specified against, and nothing in the four vendored
modules reads or writes a cache or touches the filesystem.

Consequences for operant:

- There is **no jcode path string anywhere in this directory** (verified by
  grep), and no cache-path code to repoint.
- `theme_mode()` defaults to `ThemeMode::Dark`, which is byte-identical to
  operant's current behaviour. Light-terminal adaptation is inert until
  something calls `set_theme_mode(ThemeMode::Light)`.
- Porting `theme_detect` later is a separate, self-contained job: it depends on
  `jcode_tui_style::{ThemeMode, Palette, set_palette, set_theme_mode, palette,
  theme_mode}` (all present here) plus a config-dir helper. If ported, its cache
  belongs under operant's `~/.operant/` (e.g. `~/.operant/cache/`), and
  `tempfile` would need to be added as a dev-dependency of `operant-cli` (its
  `temp_env`/`tempdir` usage is the only extra dep in that file).

## Known jcode residue (deliberate)

| Where | What | Why left |
|---|---|---|
| file headers (all 6) | "Vendored from jcode … MIT … Jeremy Huang" | Mandatory attribution. `palette.rs:6` and `mod.rs:2-4` additionally name the jcode symbol/path they replaced, which is why a grep for `.jcode` still hits two header comments — there is no jcode path in any *code* line. |
| `theme.rs:96-99` | comment referencing "the TUI run loop" spinner's `STATUS_SPINNER_ONLY_INTERVAL` | Describes an operant-side constant that does not exist yet; harmless as a note for whoever wires the spinner fast path. |

## Verification

The module compiles and its tests pass, in both shapes that matter: as a library
surface, and as a **private module of a binary** (which is what rustc sees inside
`operant-cli`, since `main.rs` declares `mod tui;`).

Verified out-of-tree against the exact same bytes, via a scratch crate whose
`src/tui/vendor/style` is a **symlink to this directory** and whose module tree
mirrors the real one:

- `cargo test` → **50 passed; 0 failed; 0 ignored**, including the two
  load-bearing invariants
  `palette::default_palette_is_frozen::every_role_keeps_its_hand_tuned_default`
  and `…::unconfigured_palette_resolves_to_the_hand_tuned_table`, plus
  `palette::buffer_tests::default_palette_leaves_the_frame_untouched` (an
  unconfigured palette is a byte-identical no-op).
- `cargo clippy --all-targets -- -D clippy::unwrap_used -D clippy::expect_used`
  → **0 errors, 0 warnings** (also the source of the `#[allow(dead_code)]`
  decision above: the unannotated build emitted 101 warnings).
- `rustfmt --edition 2024` clean on all five modules.

Once `tui/mod.rs` declares `mod vendor;` and `tui/vendor/mod.rs` declares
`pub mod style;`, the in-repo commands are:

```bash
source scripts/dev-env.sh
./scripts/check.sh check -p operant-cli --bin operant
./scripts/check.sh test  -p operant-cli --bin operant -- tui::vendor::style
```

At the time of this port those two commands could not complete: the working
tree carries unrelated WIP that does not compile (see the handover note). None
of those errors are in this directory, and cargo reported nothing at all about
`vendor/style` — expected, since nothing references it until the peer adds
`pub mod style;`.
