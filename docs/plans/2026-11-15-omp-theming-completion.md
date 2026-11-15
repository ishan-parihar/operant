# Theming completion — the engine is built, the doors are missing

Dated-frozen: published `2026-11-15`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**NOTHING IN THIS DOCUMENT IS IMPLEMENTED.** It is an outline. Say go and the first
tranche runs.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All reference paths are root-relative to that checkout.

**Baseline measured at `origin/main` `df275f5b`** — not at the working tree. That
distinction is the first finding below.

Predecessors, both **complete**:

- `docs/plans/2026-11-14-omp-theming-engine-import.md` — U1–U9, landed. The colour
  vocabulary, the markdown/syntax/diff roles, the bar fill.
- `docs/plans/2026-11-15-omp-bar-styles-import.md` — U10/U11/U12/U14, landed as
  `iter-2367`, `iter-2370`, `iter-2372`, `iter-2382`. The separator vocabulary, the
  eight styles, the caps.

---

## Resume here

Both predecessors are done and closed. This plan is the third pass, and its premise is
different: **the engine is no longer the gap. The doors are.**

---

## Headline finding

**Every capability the operator says is missing is shipped, and none of it is reachable
from the UI.**

Measured at `origin/main` `df275f5b`, not inferred:

| The complaint | What is actually in the tree | Evidence |
|---|---|---|
| "I cannot change the bar style" | **Eight separator styles exist**, with caps and a config key | `status_line/style.rs:41-58` (8 variants), `:75-86` (`key()`), `config.rs:1498` (`bar_style`), `tui_app.rs:99-114` (installed, unknown warns) |
| "the theming engine is boring" | **43 semantic roles** are writable through `[tui.colors]` | `vendor/style/palette.rs:196-242` |
| "I only have the colour scheme" | the colour scheme is the only thing `/theme` can reach | `theme_screen.rs:244-274` — Enter returns a name; nothing else is editable |

The bar style is real and it works. It is set in exactly one place: a commented line in
the example config (`operant.example.toml:477`, `#bar_style = "dot"`). There is no
settings row, no `/theme` tab, no `/bar` command. **A capability whose only control is a
commented-out TOML line is not a feature the operator has.**

The same shape explains "not ample theming options": the palette ceiling is eight
because `ColorPalette` is eight hand-written `const fn` (`theme_colors.rs:98-288`) and
there is no theme file format — `grep -rniE "themes/|theme\.toml|theme\.json|themes_dir|
load_theme" crates/` returns nothing outside the web UI. OMP ships **100** bundled
palettes (`packages/tui/src/theme/defaults.json`, counted: 100 keys) plus a user theme
directory (`packages/tui/src/theme/loader.ts:26-42`).

### The load-bearing observation

**The vocabulary is complete. The readers are complete. Only the delivery is missing —
and that is the cheapest possible gap to close.**

The colour pipeline is genuinely built: one write path (`theme_colors::set_active_theme`),
one per-frame substitution pass (`theme_mode::adapt_buffer_for_display`, called once at
`operant_ui/mod.rs:2527`), 36 of 43 roles reachable by a live reader (measured below).
This is not a half-built engine that needs finishing. It needs three doors and a filing
cabinet.

---

## What is measured, and how

Two independent counts, because a single method was wrong first and the correction is
part of the evidence.

**Role reachability — 36 of 43 have a live reader; 7 are writer-only.**
Method: for each of the 43 `Role` variants, is there (a) a call site of an accessor that
returns it outside `vendor/`, or (b) a production literal whose triple equals its frozen
default (which `configured_native_color` at `palette.rs:464-481` would rewrite)?

| | count | roles |
|---|---|---|
| reachable by an accessor | 25 | incl. all 9 `Syntax*` through one indirect call (`markdown_render_support.rs:633` → `syntax_color(role_for_scope_stack(..))`) |
| reachable only by a literal the buffer pass rewrites | 20 | `Warning` ×37 sites, `ToolDiffContext` ×33, `Info` ×23, `Success` ×19, `Border` ×19, `Queued` ×18, … |
| **union — reachable** | **36** | |
| **writer-only** | **7** | `Ai`, `Tool`, `System`, `Pending`, `HeaderIcon`, `HeaderName`, `MdListBullet` |

A first pass reported 16 and then 7, because it matched `Role::Syntax*` literals on the
call line and missed that `syntax_color` is generic over `Role` — the role arrives from
`role_for_scope_stack`, not from the call site. The correction is why the count is stated
with its method: a reader who re-runs the naive version gets a different and wrong answer.

The 7 writer-only roles are worse than a hardcode. `[tui.colors] ai = "#ff0000"` is
accepted, validated, stored, re-applied after a `/theme` switch — and repaints nothing,
because `ai_color()` has zero call sites outside `vendor/` and no widget emits
`(129,199,132)`.

**Colour literals — 561 occurrences in production TUI code, 370 unreachable.**
Method: `rgb(...)` / `Color::Rgb(...)` outside `vendor/`, `theme_colors.rs`,
`color_depth.rs`; reachable = the triple equals an overridden role's frozen default.

| | count |
|---|---|
| literal occurrences | 561 |
| reachable (triple coincides with a role default) | 191 |
| **unreachable by any theme mechanism** | **370** |
| theme-aware reads (`theme_colors::*`) | 444 |

The 191/370 split is not a design property, it is a coincidence property: a literal is
themeable only if its triple happens to equal a role's frozen default. The status bar's
amber `rgb(255,193,7)` is repaintable *only because* it collides with `Role::Queued`'s
default — the call site names nothing, and `seed_never_repaints_an_operant_value`
(`theme_colors.rs:847-881`) exists to defend that collision.

---

## Seam sheet — what crosses a boundary today

| # | Field / vocabulary | Producer | Receiver | Live carrier | Population | Duplicate owner |
|---|---|---|---|---|---|---|
| T1 | `ColorPalette` (18 fields, 8 themes) | `theme_colors.rs:98-288` | 14 accessors, 444 call sites | `RwLock<ColorPalette>` | populated | — |
| T2 | `Role` (43) | `palette.rs:148-305` | `configured_native_color` + 25 accessors | `RwLock<Option<Palette>>` | 43/43 seeded or literal-seeded | — |
| T3 | `[tui.colors]` overrides | `config.rs:1403` | `apply_role_overrides` | `ROLE_OVERRIDES` cell | live | — |
| T4 | `SeparatorStyle` (8) | `status_line/style.rs:41-58` | `separator()` → `push_separator` | `STYLE` cell, one writer | **populated but config-only** | — |
| T5 | `bar_style` config key | `config.rs:1498` | `tui_app.rs:104` (startup only) | TOML | **populated; no UI writer** | — |
| T6 | a bar-style control in the UI | — | — | — | **DOES NOT EXIST** | — |
| T7 | a theme file | — | — | — | **DOES NOT EXIST** | — |
| T8 | `MeterStyle` glyphs | `meter.rs:31-34` | 4 call sites | one `const` | no user choice | — |
| T9 | modal / frame border glyphs | `overlays/layout.rs:398-401` `Borders::ALL` | every dialog | hardcoded | no user choice | — |
| T10 | 370 unreachable literals | 88 call-site files | — | — | frozen at compile time | — |

Verified counts, not copied: 43 = 43 = 43 across the enum, `ALL_ROLES`, `key()`,
`default_rgb()`. `SeparatorStyle::ALL` is 8 and `key()` is 8.

---

## Three ledgers

### Ledger A — what OMP does (verified)

1. **Themes are data files.** `ThemeJson` (`theme/schema.ts:41-83`) = `vars` + `colors`
   + `export` + `terminal` + `symbols`. 100 bundled palettes in `defaults.json`; user
   files load from `~/.omp/agent/themes/<name>.json` (`loader.ts:26-42`).
2. **Validation is a boundary, not a hope.** `validateThemeJson()`
   (`schema-validation.ts`) rejects a custom theme with a grouped "missing required
   colour tokens" message (`loader.ts:78-101`); built-ins bypass it on purpose.
3. **The picker searches and previews.** `themePickerOptions()` sets `searchable: true`
   and `decorate: item => ({ mark: { text: item.value.slice(0,2), seed: item.value } })`
   — a swatch chip — and moving the selection calls `onPreview`
   (`overlays/theme-selector.ts:14-25, 60-62`).
4. **Bar shape is a registry of style objects, not a glyph table.** `ComposerStyle`
   (`components/composer/types.ts:78-113`) is a contract with layout metadata
   (`sideBorders`, `verticalChrome`, `statusAttachment`, `bottomBar`) plus `renderTop`,
   `renderRow`, `renderBottom`. Eight built-ins are registered in `registry.ts:12-21` and
   listed in `overlays/composer-shape-registry.ts:6-50`: `band`, `box`, `claude`, `pi`,
   `borderless`, `rule`, `field`, `rail`. Extensions can register more
   (`registry.ts:44-56`).
5. **Separator style and symbol set are separate axes.** `getSeparator()`
   (`status-line/separators.ts:4-41`) names the *shape*; the symbol preset
   (`theme/symbols.ts:1505-1509`: `unicode` / `nerd` / `ascii`, 277 `SymbolKey` entries)
   decides which codepoints draw it.
6. **Six bar presets** arrange segments (`status-line/presets.ts:4-105`).

### Ledger B — what operant does (verified, at `df275f5b`)

1. Eight const palettes × 18 fields; `/theme` applies one, persisted to `settings.json`
   (`app/providers.rs:563-585`), not to `operant.toml`.
2. 43 roles, one write path, one per-frame substitution pass. 36 reachable, 7 writer-only
   (measured above).
3. Eight separator styles with caps, installed once at startup, warned on an unknown
   value. Default `dot` — byte-identical to the pre-key look, deliberately not OMP's
   `powerline-thin` (`style.rs:26-35` records why: six ` · ` sites are prose).
4. No theme file format. No theme directory. No user-authored palette.
5. `/theme` has no search and no live preview: arrow keys move a cursor, Enter applies
   (`theme_screen.rs:244-274`).
6. `settings_screen` has a generic `Enum` row mechanism and 7 rows
   (`settings_screen/state.rs:253-310`); none of them is a bar style.

### Ledger C — the delta, and its consequence

| Capability | OMP | Operant | Consequence |
|---|---|---|---|
| colour vocabulary | 62 fg + 7 bg, data-defined | 43 roles, complete writer | adequate; nothing to build |
| palette count | 100 bundled + user files | **8, no file format** | **the "boring" complaint** |
| bar separator style | 7 styles, two axes | **8 styles, one axis** | at parity; unreachable from the UI |
| bar preset / segment list | 6 presets | none (no segment list) | not needed — see "not built" |
| bar style in the UI | settings submenu | **commented TOML line** | **the "cannot change it" complaint** |
| picker | searchable, live preview | neither | an 8-row list, no preview |
| unreachable literals | n/a | **370** | the long tail |

**The delta's shape: operant does not need a bigger vocabulary or a new engine. It needs
three doors and a filing cabinet.** Every unit below is wiring or data, not mechanism.

---

## Units, in the order each is shippable alone

### U1 — the bar style gets a control in the settings screen

The smallest unit that answers the operator's actual complaint, and the falsifier for the
rest: if a control nobody can find is the whole problem, this closes it in one change.

- **Exists today:** `SeparatorStyle` (8 variants), `bar_style` in TOML, installed at
  startup (`tui_app.rs:104`), warned on a typo. `SettingsScreen` has `SettingKind::Enum`
  and 7 rows; `apply_and_save` has a per-field arm pattern that already writes to
  `AppConfig` and persists with `persist_config_key_at` (`state.rs:122-160`).
- **Mechanism keeping it open:** the value has exactly one writer — a human editing TOML.
  Nothing in the running TUI can set it, so the operator's mental model ("I can only
  change the colour scheme") is correct.
- **Least change:** one `SettingsEntry` with `SettingKind::Enum { options: <the 8 keys> }`
  in `state.rs::all_entries`, one `bar_style: String` field on `SettingsScreen`, one arm
  in `keys.rs` that sets it, and one arm in `apply_and_save` that writes
  `config.tui.bar_style`, persists `tui.bar_style`, and calls
  `SeparatorStyle::set_style(Some(parsed))` so the change lands without a restart.
  Options must come from `SeparatorStyle::ALL` + `key()`, never a second literal list —
  `theme_screen.rs:85-95` records what a duplicated name list costs.
- **Files:** `settings_screen/state.rs`, `settings_screen/keys.rs`,
  `settings_screen/render.rs` (one row), `tui_app.rs` (reuse the existing install).
- **Size:** S — ~35 lines, no new module.
- **Guard:** a test asserting the settings screen's bar-style row lists exactly
  `SeparatorStyle::ALL` in order and that the current value round-trips; plus one test
  that an unknown value surfaces the operator's choice as a warning rather than silently
  drawing `dot`. Mutation-prove both by removing the row and by hardcoding the options.
- **Not built:** a dedicated `/bar` command; a visual preview of each style; a second
  axis (symbol set).

### U2 — a theme file format, and the palettes that come with it

The answer to "not ample theming options". Eight is a ceiling because the palette is
compiled in; a file format removes the ceiling permanently and makes every future palette
a data change.

- **Exists today:** `ColorPalette` with 18 fields; `for_theme` a `match` over eight names
  (`theme_colors.rs:71-83`); `NAMES` the one list (`:86-95`). `[tui.colors]` already
  parses `role = "#rrggbb"` and validates each entry (`palette.rs:376-393`).
- **Mechanism keeping it open:** no `Deserialize`, no reader, no directory, no listing.
  A palette can only exist if someone writes a `const fn` and recompiles.
- **Least change:** a theme file is **the `[tui.colors]` shape with a name** — the join
  key already exists. Read `<config dir>/themes/<name>.toml` holding
  `[colors] role = "#rrggbb"`, list the directory beside `ColorPalette::NAMES`, apply by
  seeding the active palette and then merging the file's pairs through the existing
  `from_pairs` + `apply_role_overrides` path. No new vocabulary, no new substitution
  mechanism, no second colour type.
- **Files:** one new `theme_files.rs` (~120 lines), `theme_screen.rs` (list merge),
  `theme_colors.rs` (apply), `operant.example.toml` (document the directory).
- **Size:** M.
- **Guard:** (1) a fixture theme file, loaded, changes a rendered cell — assert the cell,
  not the file; (2) an unknown role in a file is skipped with a warning and the remaining
  roles still apply (the existing `from_pairs` contract, pinned once for the file path);
  (3) a malformed file leaves the previous theme in force and says so; (4) a built-in name
  is never shadowed by a file (OMP's precedence, `loader.ts:26-42`).
- **Not built:** light/dark slots (operant ships self-contained palettes); a file watcher;
  per-session accent derivation.

**Ruling needed before U2 is sized finally — see "Rulings".**

### U3 — the seven writer-only roles

Seven keys that accept a value and repaint nothing are worse than seven hardcodes: the
config surface promises a knob and the knob is not attached.

- **Exists today:** `Role::Ai`, `Tool`, `System`, `Pending`, `HeaderIcon`, `HeaderName`,
  `MdListBullet` are declared, seeded, listed in `ALL_ROLES`, accepted by `from_pairs` —
  and read by nothing.
- **Mechanism keeping it open:** the accessor exists (`ai_color()`, `tool_color()`,
  `system_message_color()`, `pending_color()`, `header_icon_color()`,
  `header_name_color()`) with zero call sites, and no production literal equals their
  defaults.
- **Least change — two options, pick by evidence, not taste:** (a) delete the seven roles,
  their accessors, their seed rows and their `from_pairs` keys — deletion is smaller and
  honest; or (b) point one existing widget per role at the accessor — but only where a
  widget is already painting the right thing in the wrong way. `Ai` and `Tool` are
  candidates for (b): `ui_messages.rs` paints assistant and tool chrome from
  `theme_colors::*`, so the roles have an obvious home. The other five have no home and
  should be deleted.
- **Files:** `palette.rs` (enum, `key()`, `default_rgb()`, `ALL_ROLES`),
  `theme_colors.rs` (`roles_for_field`, `LITERAL_SEEDED_ROLES`), `theme.rs` (accessors).
- **Size:** XS for deletion; S for the two wirings.
- **Guard:** one test that every role in `ALL_ROLES` has at least one live reader —
  grep-shaped over `crates/operant-cli/src` outside `vendor/`, so a role added later
  without a reader fails the build. This is the guard the whole class of finding needs.
- **Not built:** a ninth markdown role, a syntax-theme picker.

### U4 — `/theme` gains search and a live preview

Cheap parity, and it makes U2's larger list usable: an operator cannot choose among forty
palettes with a cursor and no preview.

- **Exists today:** arrow keys move a cursor; Enter applies (`theme_screen.rs:244-274`);
  the swatch is read from the palette itself so it cannot drift (`:125-146`).
- **Mechanism keeping it open:** `select_next` / `select_prev` mutate an index and nothing
  else; applying is the Enter arm only.
- **Least change:** call the same apply path on selection change (preview, not persist),
  restore the prior theme on Esc, and add a filter over `themes` by name. `settings_screen`
  already has a searchable list to copy the shape from.
- **Files:** `theme_screen.rs`, `app/key_handling.rs` (Esc restores).
- **Size:** S.
- **Guard:** a test that moving the selection changes the rendered swatch row and that Esc
  restores the theme that was active at open.
- **Not built:** a colour-swatch grid, an ANSI preview pane.

### U5 — the 370 unreachable literals, surface by surface

Not a unit so much as a standing rule: the tail is 370 occurrences across 88 files and
nobody should sweep it.

- **Least change per surface:** name the role, point the helper at it, regenerate the
  affected `operant-style-v1` golden in the same commit. Start with the worst:
  `diff_viewer/render.rs:466` `.bg(Color::Rgb(150,30,30))` and `:474`
  `.bg(Color::Rgb(30,130,30))` — the word-level diff backgrounds, which are unreachable
  and are the exact red/green pair the `deuteranopia` palette exists to fix. Then the
  markdown chrome: `md_dim_color()` (`operant_markdown/mod.rs:548-550`) alone drives
  blockquotes, list bullets, ordered markers, thematic breaks, heading rules and code-block
  frames.
- **Guard:** the existing 68 scenario `*.style.txt` goldens. `theme-screen.scenario.json`
  is the cheapest palette-wide canary; `status-row` is the cheapest bar canary.
- **Not built:** a sweep. Per surface only.

---

## Size table

| Unit | File(s) | Size | Signal it worked | Closable here? |
|---|---|---|---|---|
| U1 bar style control | `settings_screen/*`, `tui_app.rs` | **S** | the settings row lists 8 styles; choosing one changes the drawn bar with no restart | **yes** |
| U2 theme file format | `theme_files.rs` (new), `theme_screen.rs`, `theme_colors.rs` | **M** | a fixture file changes a rendered cell; a bad role is skipped, not fatal | **yes, after the ruling** |
| U3 writer-only roles | `palette.rs`, `theme_colors.rs`, `theme.rs` | **XS/S** | a role with no reader fails the build | **yes** |
| U4 picker search + preview | `theme_screen.rs`, `key_handling.rs` | **S** | moving the selection repaints; Esc restores | **yes** |
| U5 unreachable literals | per surface | **S each** | the surface's golden changes only where intended | ongoing |

Size is by files, state transitions and guards touched — not effort points. Evidence read
for each is cited inline above.

---

## Rulings — choices only the owner can make

Recorded as exceptions with nothing to build until decided. Each changes unit boundaries,
so none is smuggled into a default.

| # | Choice | Options and consequence |
|---|---|---|
| R1 | **How many bundled palettes, and from where?** | (a) Format only, ship zero extra palettes — smallest, but the operator still sees eight until they hand-write one. (b) Format + a hand-mapped subset (~10) — reviewable by eye, but arbitrary. (c) Format + all 100 from OMP's `defaults.json` — the real answer to "ample", but the mapping is lossy: OMP's 69 tokens onto operant's 43 roles, and 26 OMP tokens have no target role (13 `statusLine*` vs one bar-fill role; 10 `md*`/`syntax*` partly). Recommend (c) with the unmapped tokens named in the generator, because (a) and (b) leave the complaint unanswered. |
| R2 | **TOML or JSON for the theme file?** | (a) TOML — matches the repo's config convention, and the file *is* the `[tui.colors]` shape. (b) JSON — matches OMP, so `defaults.json` can be copied mechanically, but a second format in the tree. Recommend (a); the generator emits TOML from `defaults.json` once, at build-authoring time, not at runtime. |
| R3 | **Delete or wire the five homeless roles?** | Deleting `System`, `Pending`, `HeaderIcon`, `HeaderName`, `MdListBullet` is smaller and honest; wiring them invents a surface for each. Recommend delete, with `Ai`/`Tool` wired in the same unit. |
| R4 | **Does U1 ship before U2?** | U1 answers the complaint the operator actually made, in an S. U2 answers the complaint underneath it. Recommend U1 first: it is one tranche and it proves the door-shaped diagnosis before the filing cabinet is built. |

---

## Local strengths to preserve — do not flatten these into a parity checklist

- **Colour-depth quantization is strictly ahead of OMP.** Four tiers with a redmean-
  weighted nearest-colour search and an `OPERANT_COLOR_DEPTH` override
  (`color_depth.rs:44-221`); OMP has two and delegates to `Bun.color`.
- **`deuteranopia` is a first-class theme.** OMP has no equivalent; it has a one-token
  `colorBlindMode` that HSV-shifts `toolDiffAdded` only (`loader.ts:154-159`).
- **The visual corpus asserts per-cell fg/bg** (`operant-style-v1`, 68 scenarios), so
  every unit above has a mechanical guard OMP does not have.
- **The default bar style is `dot`, not OMP's `powerline-thin`** — a deliberate divergence
  with a measured reason (`style.rs:26-35`: six ` · ` sites are prose), and it means
  adopting the key changes nothing on screen until the operator chooses.

---

## What is deliberately not built

- **A segment model.** OMP's `status-line/presets.ts` arranges a segment list; operant's
  status row is a flat `Line` and fits at 64 columns. The predecessor built and deleted
  this once (`iter-2363`) and the reason still holds.
- **A second symbol-set axis.** OMP's second set is ASCII; operant's `ascii` separator
  style already covers the unpatched-font case with one config value.
- **A composer-shape registry** (OMP's `ComposerStyle`, eight silhouettes). Operant's
  composer is one deliberate borderless band (`composer.rs:21-29`); a silhouette axis is
  a larger design change than any complaint here.
- **Per-session accent derivation** (OMP's OKLCH arc, `theme/session-color.ts`) and
  **shimmer** (`theme/shimmer.ts`). Both are named as out of reach in the predecessor and
  nothing here changes that.
- **A live-reload watcher.** A theme file read once at apply time is enough; a watcher is
  a second concurrency story for no measured gain.
- **Sweeping all 370 literals.** Per surface, as each gains a role (U5).

---

## Coordination

- **The main working tree is 7 commits ahead of `origin/main` and 71 behind, with 54
  uncommitted files from a peer.** The 7 local commits (`iter-2372`…`iter-2390`) exist on
  no remote ref. Nothing in this plan touches them; whoever owns them should push or
  branch them before the next ff.
- **The deployed binary `~/.local/bin/operant` (0.3.0, built 10:47 today) already has
  `bar_style`** — verified by running it with `bar_style = "powerline"` and finding
  U+E0B2 caps at both ends of the status row. It predates only `iter-2403` (11:46).
  An operator who reports "no bar styles" is looking at a control that does not exist, not
  at a feature that is missing.
- **`.wt-barstyle` and `.wt-theme`** are the predecessors' worktrees; both branches are
  ancestors of `origin/main`. Rule 8: not mine to rebase or delete.

## Not checked

- **Live rendering on a real terminal.** The bar-style run was through the headless
  simulator; whether U+E0B0/U+E0B1 render is a property of the operator's font.
- **Whether `theme_screen`'s eight-row list is what the operator was looking at.** The
  complaint names "colour-scheme"; if they meant per-role colour, `[tui.colors]` is the
  answer and U1 is not.
- **OMP's `defaults.json` palette quality** under operant's 43-role vocabulary. The
  mapping is mechanical; whether each imported palette *looks* right is a review, not a
  measurement.
- **The `output_style` mismatch** noticed in passing and out of scope: the settings row
  lists `default|concise|explanatory|learning` (`state.rs:265`) while `/output-style`
  cycles `auto|stream|verbose` (`app/commands.rs:1371-1387`). Two vocabularies for one
  setting.

---

## If you say go

Run **U1** first. It is one small change, it is the operator's stated complaint, and it
proves the diagnosis: the engine was never the gap, the doors were.