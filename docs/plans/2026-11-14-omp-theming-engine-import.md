# Operant theming engine — upgrade outline from the oh-my-pi (OMP) reference

Dated-frozen: published `2026-11-14`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**Resume here (2026-11-14, after U1/U2 landed):** U1 and U2 are DONE and on `origin/main`
(iter-2265, iter-2270). U3 is IN PROGRESS and its shape has changed — read "U3 redesign"
below before writing code. The original U3 as written here is NOT safe to implement as
specified.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All reference paths are root-relative to that checkout.

> **This file was lost once.** It was written, never committed, and removed by a storage
> sweep (AGENTS.md rule 11). It is restored from the session record. It is committed with
> this revision so the next sweep cannot take it again.

---

## Headline finding

**Operant had no theming engine. It had two colour tables, one of them unreachable, and
the surface an operator actually reads — assistant output — followed neither.**

OMP's theming engine is a *closed semantic vocabulary*: 62 foreground keys
(`packages/tui/src/theme/schema.ts:86-146`) plus 7 background keys (`:208-215`). Every
painted surface names a key; every theme is a data file that fills those keys; the text
layer, the syntax highlighter, the bars and the diff all read the **same** table.

Operant had, before U1/U2:

1. `theme_colors.rs` — 8 hand-written Rust palettes, read through 15 accessors by 48
   files. This was the live one.
2. `vendor/style/palette.rs` — a **22-role** semantic palette, seeded *from* (1), whose
   only configuration entry point `Palette::from_pairs` had **zero production callers**.

Meanwhile **782 colour literals** (554 `rgb(...)`, 228 `Color::Rgb(...)`) across 32 files
sit outside both tables.

### The load-bearing observation

**Markdown and code output do not follow the theme at all.**

| Surface | Operant | OMP |
|---|---|---|
| markdown body / heading / code / link | 12 hardcoded `rgb()` constants (`tui/operant_markdown/mod.rs:503-542`) | 17 `md*` theme keys (`theme/tui-adapters.ts:297-345`) |
| code syntax colours | syntect pinned to `base16-ocean.dark` (`markdown_render_support.rs:467`, `:511`; `diff_viewer/render.rs:502`) | syntect **too**, but scopes map onto 9 theme-derived `syntax*` tokens (`tui-adapters.ts:28-46`) |
| diff added / removed | hardcoded `Rgb(100,200,100)` / `Rgb(200,100,100)` (`tui/operant_ui/ui_diff.rs:10`, `:14`) | `toolDiffAdded` / `toolDiffRemoved` / `toolDiffContext` |
| bar fill + segments | no fill, no segment vocabulary (`ui_input.rs:907` `draw_status`) | `statusLineBg` + 14 `statusLine*` keys + 27 segment ids (`status-line/schema.ts:2-28`) |

`/theme` repainted the chrome and the dialogs and left every word of assistant output in
colours chosen at compile time. **The highlighter crate is not the gap** — operant already
uses syntect, exactly as OMP does. The gap is that operant never maps syntect's scopes onto
its own palette, and pins one foreign theme instead.

---

## Status

| Unit | State | Commit |
|---|---|---|
| U1 — `[tui.colors]` writer for the 22-role palette | **DONE** | iter-2265 |
| U2 — theme selection total, default is a real theme | **DONE** | iter-2270 |
| U3 — `md*` / `syntax*` / diff role keys | **IN PROGRESS — redesign required** | — |
| U4 — route markdown helpers through the roles | pending | — |
| U5 — map syntect scopes onto theme syntax roles | pending | — |
| U6 — theme the diff colours | pending | — |
| U7 — bar fill + separator vocabulary | pending | — |
| U8 — segment model + presets + overflow degrade | pending | — |
| U9 — one meter primitive | pending | — |

### What U1 actually landed

`Palette::from_pairs` was a dead seam: a parser for `role = "#rrggbb"` with no caller.
Closed by:

- `crates/operant-core/src/config.rs`: `[tui.colors]` as `BTreeMap<String,String>` on
  `TuiSettings`, defaulting empty.
- `crates/operant-cli/src/tui/theme_colors.rs`: a `ROLE_OVERRIDES` cell +
  `set_role_overrides()` writer, and `apply_role_overrides()` called from
  `set_active_theme` — the ONE write path — so a `/theme` switch re-applies the overrides
  instead of dropping them. The merge carries over the roles the operator named AND got
  right, because `from_pairs` returns defaults for everything else and installing it
  wholesale would wipe the seed.
- `crates/operant-cli/src/tui/adapter_types/tui_app.rs`: installs the overrides before the
  first frame.
- `operant.example.toml`: advertises the section.

Also fixed in passing (one line, flagged in the commit): `gateway_commands.rs:3767` read
`telegr[REDACTED](&vendor)` — a garbled identifier present at `origin/main` that broke the
ENTIRE `operant-cli` test target with E0425. Restored to `telegram_bot_commands`.

### What U2 actually landed

Both shipped defaults named a theme that does not exist: `operant.example.toml` **and**
`impl Default for TuiSettings` set `theme = "opencode"` — a provider id. `for_theme`
matched eight names with `_ => default_theme()`, so anyone who never picked a theme
rendered the silent fallback and was told nothing. `Default for TuiSettings` was the worse
half: it is what a config with no `[tui]` section deserializes to.

`for_theme` now returns `Option<Self>`, `ColorPalette::NAMES` is the one list, and
`set_active_theme` warns naming the bad value. `theme_screen.rs`'s duplicate `THEME_NAMES`
array is now `ColorPalette::NAMES`.

---

## U3 redesign (READ BEFORE CODING U3)

The original U3 said "extend `Role` with `md*` / `syntax*` / diff keys, each default equal
to the literal it replaces." **That is not safe as written**, for two measured reasons.

### Reason 1 — the buffer pass attributes a cell by EXACT default match

`configured_native_color` (`vendor/style/palette.rs:345-362`) walks `ALL_ROLES` and takes
the **first** role whose frozen default equals the cell's colour. Two roles sharing a
default make attribution ambiguous: `.find()` picks whichever is declared first, and the
other can never be configured.

Three of the proposed defaults collide with an existing role:

| Proposed role | Default | Collides with |
|---|---|---|
| `ToolDiffAdded` | `(100,200,100)` | `Role::Success` |
| `MdHr` | `(100,100,110)` | `Role::Border` |
| `MdListBullet` | `(186,139,255)` | `Role::Accent` |

And three collide with each other: `MdLinkUrl` / `MdQuote` / `ToolDiffContext` all want
`(100,100,100)`, and `SyntaxOperator` / `SyntaxPunctuation` both want `(212,212,212)`.

### Reason 2 — measured live literal counts

A new role whose default equals a live literal repaints every un-migrated cell carrying it
the moment the operator configures that role. Counted across `crates/operant-cli/src/`:

| Literal | Source sites | Meaning |
|---|---|---|
| `(100,200,100)` | **22** | `Role::Success`'s default, used as "added" in 10 files |
| `(100,100,110)` | **21** | `Role::Border`'s default, used as borders in 6 files |
| `(186,139,255)` | 6 | `Role::Accent`'s default |
| `(100,100,100)` | 4 | `md_dim_color()` |
| `(180,180,180)` | 2 | `code_fg()` |
| `(200,200,195)` / `(200,155,75)` / `(120,180,240)` / `(45,45,45)` / `(200,100,100)` | 1 each | the other markdown literals |

So `ToolDiffAdded` cannot take `(100,200,100)`: it would collide with Success, and either
configuring diff-adds recolors 22 success cells or configuring success recolors every diff.

### The shape U3 must take

1. **Give every new role a UNIQUE default** — verified unique against all 22 existing
   defaults and against each other. OMP's `dark.json` values work for most; nudge only
   where they collide. Recompute and re-verify the table with a script rather than trusting
   a copied one.
2. **`ToolDiffAdded` / `ToolDiffRemoved` must NOT reuse the current diff literals.** Their
   defaults have to be unique, which means the *first* frame after U3 changes the diff
   colours slightly. That is a deliberate, one-time appearance change: it is the cost of
   making them themeable at all. Say so in the commit and regenerate the affected
   `operant-style-v1` goldens in the same commit.
3. **`seed_covers_every_role` forces every role to be seeded from a `ColorPalette` field**
   (`theme_colors.rs:850-873`). Twenty new roles therefore need twenty new theme fields, or
   the guard must learn a second category: roles seeded from a fixed literal rather than
   from a theme field. Prefer the second — adding 20 fields to 8 hand-written palettes is
   the larger change, and these roles are *supposed* to be theme-independent until a theme
   claims them.
4. **Keep `seed_never_repaints_an_operant_value` green.** It already fails the build when a
   fourth seeded role collides with a theme value; it is the guard that caught reasons 1
   and 2 above, and it must not be weakened.

---

## Seam sheet — what crosses a boundary today

| # | Field / vocabulary | Producer | Receiver | Live carrier | Population | Duplicate owner |
|---|---|---|---|---|---|---|
| S1 | `ColorPalette` (18 fields) | `theme_colors.rs` | 15 accessors, 48 files | `RwLock<ColorPalette>` | populated, 8 values | `theme_colors.rs` only |
| S2 | `Role` (22 roles) | `vendor/style/palette.rs:50-73` | `adapt_buffer_for_display` | `RwLock<Option<Palette>>` | seeded from S1 | 2nd table |
| S3 | `Palette::from_pairs` | `palette.rs:257` | **now `apply_role_overrides`** (U1) | — | **CLOSED by U1** | — |
| S4 | `tui.theme` config key | `config.rs` | `app/init.rs`, `app/providers.rs` | TOML + `settings.json` | populated; validated by U2 | — |
| S5 | `md*` / `syntax*` semantic keys | — | — | — | **DOES NOT EXIST** | — |
| S6 | markdown colour constants | `operant_markdown/mod.rs:503-542` | `markdown_render_lazy.rs` (~40 sites) | `const fn` | populated, **theme-invariant** | 1 |
| S7 | syntect theme name | `markdown_render_support.rs:467` | `markdown_render_lazy.rs:478`, `diff_viewer/render.rs:502` | `LazyLock<ThemeSet>` | `"base16-ocean.dark"` always | 3 sites, 1 literal |
| S8 | bar segments | — | — | — | **DOES NOT EXIST** | — |
| S9 | `ColorDepth` + quantizer | `color_depth.rs` | `theme_colors.rs` (one call) | `OnceLock<ColorDepth>` | populated | 1 — **operant strength** |
| S10 | symbol / glyph preset | scattered literals | — | — | no preset concept | many |

---

## Three ledgers

### Ledger A — what OMP does (verified)

1. **One vocabulary, data-defined.** `ThemeJson` (`theme/schema.ts:41-83`) = `vars` +
   `colors` + `export` + `terminal` + `symbols`. 100 bundled themes, each a pure data file;
   custom themes load from a user directory (`theme/loader.ts:26-42`).
2. **Reference resolution.** A colour may be a hex, a 256 index, or the *name of a var*;
   `resolveVarRefs` (`theme/color.ts:63-80`) chases the chain and throws on a cycle.
3. **Light/dark is derived, not declared.** `Theme` classifies itself from the luma of
   `statusLineBg` (`theme/theme-class.ts:215-217`), which decides how the `""`
   (terminal-default) sentinel resolves (`theme/color.ts:104-108`).
4. **Depth degrades once, at the edge.** `detectColorMode` returns `truecolor` |
   `256color` only (`theme/color.ts:13-19`); `colorToAnsi` converts per colour with a
   256-entry cache (`theme/color.ts:28-46`).
5. **Bars are a segment algebra.** 27 segment ids (`status-line/schema.ts:2-28`), 7 presets
   (`status-line/presets.ts:4-105`), 7 separator styles (`status-line/separators.ts:4-41`).
   The bar fill is one key: `theme.getBgAnsi("statusLineBg")`
   (`status-line/component.ts:3135`).
6. **Powerline caps reuse the fill as ink**: `bgAnsi.replace("\x1b[48;", "\x1b[38;")`
   (`status-line/component.ts:3333`).
7. **Overflow is an ordered degrade**: shrink session name to 8 cells → pop right segments
   right-to-left → shrink path over ≤8 re-renders → drop left segments right-to-left, never
   dropping path first (`status-line/component.ts:3245-3320`).
8. **Text styling is a theme object of closures**: `getMarkdownTheme()` returns
   `{heading, code, link, linkUrl, codeBlock, codeBlockBorder, quote, quoteBorder, hr,
   listBullet, bold, italic, ...}`, each closing over one theme key
   (`theme/tui-adapters.ts:297-357`).
9. **Syntax colours are theme-derived.** `getHighlightColors(theme)` maps 11 categories onto
   `theme.getFgAnsi(...)`, cached per `Theme` instance (`tui-adapters.ts:28-46`).
10. **The highlight cache is keyed on theme identity** and cleared whenever the `Theme`
    instance changes, because depth is baked into the cached escape (`tui-adapters.ts:60-96`).
11. **Optional tokens stay optional** — omitted tokens resolve to `\x1b[39m` while
    `hasColor()` reports them unset (`theme-class.ts:228-235`), so prose can deliberately
    inherit the terminal default.
12. **Streaming highlighting is stateful and parity-pinned**: a Rust test asserts chunked
    output is byte-identical to whole-text (`highlight.rs:868-882`).

### Ledger B — what operant does (verified)

1. 8 palettes × 18 fields, hand-written as Rust `const fn`. Selection is now explicit (U2).
2. Process-global `RwLock<ColorPalette>`, written only by `set_active_theme`, read by 15
   accessors.
3. `set_active_theme` quantizes to the detected depth (`color_depth.rs:338-361`), seeds the
   22-role palette from the *unquantized* source, then re-applies `[tui.colors]` (U1).
4. `ThemeMode` is **pinned to `Dark` unconditionally**, deliberately: operant ships
   self-contained palettes, so the light adaptation would double-adapt.
5. A per-frame substitution pass runs over the whole buffer as the last colour transform
   (`operant_ui/mod.rs` → `theme_mode.rs:256`).
6. Markdown: 12 `const fn` colour helpers, theme-invariant, ~40 call sites in
   `markdown_render_lazy.rs`.
7. Syntax: `SYNTAX_SET` / `THEME_SET` are `LazyLock` globals; `"base16-ocean.dark"` is
   indexed at three sites.
8. Bars: `draw_status` is one imperatively composited `Line` — no fill, no separator
   vocabulary, no segment abstraction.
9. Meter widget is one shape (`█`/`░`), duplicated at `context_viz.rs:96-115`,
   `usage_overlay.rs:370`, `markdown_wrap.rs:545`.
10. Guard already in place: the visual corpus captures per-cell fg/bg as
    `operant-style-v1` goldens, gated by `_run.py verify`.

### Ledger C — the delta and its consequence

| # | Delta | Consequence |
|---|---|---|
| D1 | ~~22-role vocabulary had no writer~~ | **CLOSED by U1** |
| D2 | ~~shipped default theme did not exist~~ | **CLOSED by U2** |
| D3 | No `md*` / text keys | `/theme` cannot repaint a single word of assistant output |
| D4 | No `syntax*` keys; one foreign syntect theme pinned | code blocks keep a VSCode-dark palette under every operant theme, including `light` |
| D5 | Diff colours hardcoded | added/removed ignore the theme and the `deuteranopia` palette |
| D6 | No bar fill / segment model | no powerline, no per-segment colour, no preset, no overflow degrade |
| D7 | 782 literals outside both tables | the remaining backlog |
| D8 | No per-theme invalidation of the highlight cache | once D4 lands, `/theme` would keep painting stale syntax colours |

**Local strengths to preserve — do not flatten these into a parity checklist:**

- **Colour-depth quantization is strictly ahead of OMP.** 4 tiers with a redmean-weighted
  nearest-colour search and an `OPERANT_COLOR_DEPTH` override (`color_depth.rs:44-215`);
  OMP has 2 tiers and delegates to `Bun.color`.
- **`deuteranopia` is a first-class theme**; OMP has no equivalent.
- **The visual corpus already asserts per-cell fg/bg**, so every unit has a mechanical guard.
- **`StyleRole` / `FillRole` already exist** (`operant_render_core/model.rs:21-46`) — the
  vocabulary is half-built; it stops at the ratatui adapter.

---

## Remaining units

### U4 — route the markdown helpers through U3's roles

The 12 `const fn` in `operant_markdown/mod.rs:503-542` become role reads. Signatures stay,
so no call site changes. Guard: `cargo test -p operant-cli markdown`, plus the
`operant-style-v1` goldens regenerated **deliberately in the same commit** — the golden diff
is the review.

### U5 — map syntect scopes onto the theme instead of pinning a foreign theme

Keep syntect for parsing; replace `"base16-ocean.dark"` with the `syntax*` roles from U3.
Two things OMP gets right that operant must copy:

- the highlight cache **must** be invalidated when the theme changes, because depth is baked
  into the cached escape (`tui-adapters.ts:60-96`) — D8;
- streaming fences need a stateful highlighter with a **chunked-equals-whole-text** parity
  test (`highlight.rs:868-882`).

Guard: render one Rust snippet under `dark` and under `light`, assert the keyword colour
differs — the defect, stated as a check — plus a chunked-vs-whole parity test.

### U6 — theme the diff colours

`ui_diff.rs:10` / `:14` → `Role::ToolDiffAdded` / `ToolDiffRemoved`, plus
`ToolDiffContext`. Note the U3 redesign constraint: their defaults must be unique, so this
is a one-time appearance change. OMP also wraps changed words in inverse video for 1-for-1
replacements (`chrome/diff.ts:57-101`) — worth copying, but separate.

### U7 — bar fill + separator vocabulary

Add `statusLineBg` to the background role set (mirroring OMP's `ThemeBg`,
`theme/schema.ts:208-215`) and transcribe the separator table from
`status-line/separators.ts:4-41`. In ratatui the `useBgAsFg` cap trick disappears — caps
take the neighbour's colours directly.

### U8 — segment model + presets + overflow

`draw_status` becomes `Vec<Segment>` plus the 7 presets from `status-line/presets.ts:4-105`
and the ordered overflow degrade from `component.ts:3245-3320`. Operant's existing status
content maps onto segment ids rather than being rewritten. Guard: a width sweep at
200/120/80/60 columns.

### U9 — one meter primitive

`█`/`░` duplicated at three sites becomes one `meter(filled, empty, width, style)`. Guard:
the existing tests that assert `█`/`░` keep passing **unchanged**.

### Out of reach (named, with the reason)

| Gap | Why |
|---|---|
| Importing OMP's 100 bundled themes | Needs a Rust theme-file format first (U1 decided the override shape; a file format is its own unit). |
| OMP's session accent OKLCH arc (`theme/session-color.ts:176-253`) | Needs OKLCH + gamut-cusp; largest single item in the reference. |
| Native / TSP terminal rendering (`packages/tui/src/native/*`) | Operant is ratatui; the ANSI fallbacks are the dead path here. |
| Shimmer / KITT animation (`theme/shimmer.ts`) | Operant has a decorative-animation policy already; a port is a behaviour change. |
| LaTeX / mermaid / sixel styling | Separate renderers with their own theme surfaces. |
| Deleting all 782 literals in one sweep | Mechanical but 32 files; do it per-surface as each gains a role. |

---

## Environment traps hit while executing this plan

Recorded because they cost real time and are not in AGENTS.md:

1. **`~/.local/bin/cargo` is a systemd build-slice wrapper.** In a sandbox where the user
   bus is unreachable it `exec`s `systemd-run`, which fails, and cargo never runs — the
   symptom is a log containing only `Failed to connect to user scope bus`. Fix:
   `unset XDG_RUNTIME_DIR`, which takes the wrapper's own documented plain-cargo fallback.
2. **`espeak-rs-sys` needs a writable `TMPDIR`.** make's jobserver creates a fifo in
   `$TMPDIR`; the default `/var/tmp/ishanp-agentic` is read-only under the sandbox, which
   surfaces as `jobserver mkfifo: ... Read-only file system` followed by
   `Compiled phonemes: N errors`. Fix: `TMPDIR=<workspace>/.wt-tmp`.
3. **A killed espeak build leaves a poisoned build dir** that fails with
   `source directory ... does not appear to contain CMakeLists.txt` on every later build.
   Fix: delete that one `target/debug/build/espeak-rs-sys-*/` directory.
4. **`cargo fmt --all` reformats pre-existing drift.** The tree at `origin/main` is not
   rustfmt-clean, so a repo-wide `fmt` produces an 18-file diff. Fix: run
   `rustfmt --edition 2024 --check <file>` per file you touched and revert the rest.
5. **`include_str!` resolves from the file's own directory.** From
   `crates/operant-cli/src/tui/` the repo-root example is `../../../../operant.example.toml`,
   not the `../../../` that `operant-core/src/` uses.

## Pre-existing failures at `origin/main` (NOT caused by this work)

- `gateway_commands.rs:3767` `telegr[REDACTED]` — broke the whole `operant-cli` test target.
  **Fixed in iter-2265** because no theming guard could otherwise run.
- `tui::app::commands::tests::no_palette_command_falls_into_the_catch_all_category` — fails
  at baseline with only the REDACTED fix applied. The five `/ponytail-*` commands have no
  palette category.
- 3 `operant-core` clippy warnings (`agent/turn_context.rs:331`, `agent/events.rs:786`,
  `gateway_runner.rs:2662`).
- `origin/main` is not rustfmt-clean.