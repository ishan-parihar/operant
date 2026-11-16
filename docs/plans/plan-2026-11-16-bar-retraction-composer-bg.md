# Plan — 2026-11-16: retract the `/bar` status-line half-port, theme the input area itself

Dated-frozen: published `2026-11-16`. A plan that changes is a new plan.

**STATUS: COMPLETE (iter-2469).** All three phases landed. The golden corpus
was regenerated for every scenario whose composer band color changed (70 style
goldens); text goldens were left alone because this change is color-only, and
the handful of text-golden diffs observed during regeneration were peer
staleness (help-overlay's variant rows, theme-screen, typeahead) or the known
session-hub flake, not this work.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All reference paths are root-relative to that checkout.

Predecessor: `docs/plans/2026-11-15-omp-bar-styles-import.md` (U10–U14, closed).
That plan's own U10–U14 built a *separator glyph vocabulary* and a
`[tui] bar_style` config key for the status row above the composer. The two
iterations after it (`iter-2445` direct-apply, `iter-2454` picker) polished the
command surface of that key.

## The operator's ruling (2026-11-16)

> "WTF is this /bar? What was it supposed to do? It adds a grey-line above the
> actual input-area, rather than changing the input-area style itself."

Investigation outcome, stated before the work:

1. The grey area above the input area is the **status row** (`chunks[2]`,
   `operant_ui::ui_input::draw_status`). It is not a border and not a new
   chrome element; it is the row that carries tips and affordance hints.
2. The `/bar` feature was a **half-port**. OMP has no `/bar` command and no
   "bar style" key. What OMP has is a `StatusLine` **embedded inside the
   editor component's top border** (`status-line/component.ts:3105`
   `#renderStatusLine`, `:3565` `attachToEditor`, `editor.ts:1590` the
   top-border provider). The *input area's own fill* in OMP comes from
   `EditorTheme.surfaceColor` = `theme.bgFill("userMessageBg")`
   (`theme/tui-adapters.ts:407`) — the composer band IS themed through
   `userMessageBg`.
3. Operant ported the separator-glyph leaf (U10–U14) onto a standalone
   `StatusLineBg` row, gave it a `bar_style` key, then an invented `/bar`
   command and picker. At no point did the input area itself become
   themeable: `composer::paint_band_fill` painted a hard-wired luminance lift
   of `panel_bg()` with no role, no operator key, and no theme switch could
   reach it.
4. The user's "the grey area above the input area" and "the input-area style
   itself" are therefore two different things the half-port conflated.

## The work, in phases

### Phase 1 — remove the half-port entirely

- Delete the `/bar` command arm (`tui/app/commands.rs`), its picker
  (`PickerKind::Bar`, `PickerAction::BarStyle`, `open_bar_picker`,
  `apply_bar_style`, `preview_bar_selection`, `restore_committed_bar_style`),
  the `[tui] bar_style` config field (`operant-core/src/config.rs`), its
  registry entries (`config_registry.rs` — doc row, `SettingRow`, removal
  list), the settings-screen row and writer (`settings_screen/state.rs`,
  `settings_screen.rs`), the `REGISTERED_COMMANDS` row and `CommandDef` +
  workspace category in `commands.rs`, and every `bar_command_tests` module.
- Delete `tui/status_line/{style,separator,bar}.rs` and `Role::StatusLineBg`
  (+ its `theme.rs` accessor and `theme_colors.rs` seed entry and the
  `status_line_bg` pair in all eight bundled themes).
- Restore the status row's boundary to the historical literal: one
  `SEGMENT_SEPARATOR` constant (`" · "`) in `ui_input.rs` plus a local
  `push_separator` with the same first-segment-wins semantics. The fill and
  its gate (`status_row_fills`, `add_caps`, the `status_row_is_real`
  parameter) are deleted: the row is transparent and shows only what it has
  to say.
- Repoint `operant.example.toml` (drop the `bar_style` block).

### Phase 2 — theme the input area itself

- New role `Role::ComposerBg` (default `(44, 50, 62)`) in
  `vendor/style/palette.rs` — enum, `ALL_ROLES`, `key()` `"composer_bg"`,
  `default_rgb`, `HAND_TUNED`.
- `theme_colors.rs`: seed `ComposerBg` from the SAME `panel_bg` field the
  user bubble uses (`roles_for_field("panel_bg") => &[Role::UserBg,
  Role::ComposerBg]`). The band therefore changes with every theme like the
  rest of the chrome, and `[tui.colors] composer_bg = "#rrggbb"` overrides
  it through the existing role-override path — no new config key, no new
  writer. `every_role_is_reachable_through_the_live_theming_path` covers the
  new role automatically because it enumerates `ALL_ROLES`.
- `composer.rs`: `composer_fill_style()` returns
  `Style::default().bg(theme::composer_bg())`; the luminance-lift helpers
  (`blend`, `composer_surface_rgb`) are deleted with their tests.

### Phase 3 — record, verify, ship

- This plan doc + `docs/CHANGELOG.md` entry (own entry; the shared file
  carries peers' in-flight entries — see Risks).
- `cargo fmt --all` (revert unrelated churn), `cargo check -p operant-cli
  --bin operant`, `cargo test` for the touched surfaces, full-suite failure
  set diffed against a pristine `origin/main` build, `cargo clippy
  --all-targets` no new warnings.
- Commit with an iter label from `origin/main` top + 1; push; release build;
  deploy to `~/.local/bin/operant` per AGENTS.md §7 (short target dir, mtime +
  md5 content check).

## Risks / traps

- **The shared tree is a live integration zone.** Peers edit the same files
  concurrently (observed mid-task: `kanban/db.rs` red from peer WIP,
  `session_hub.help_active` in-flight, `keys.rs::toggle_or_cycle_current`
  visibility fix uncommitted, `theme_colors.rs` `heading_accent` WIP). Every
  build for this change is verified in a clean worktree; the commit stages
  ONLY this change's files/hunks.
- **Origin moved during the task** (`485569fd` → `daa6b623`). The iter label
  is computed from the live `origin/main` at push time.
- **Golden corpus.** Two baselines (`assistant-code-block.style.txt`,
  `edit-row-diff-expands.style.txt`) carry the `rgb:161A21` fill row and
  must be regenerated via `_run.py prove <scenario>` after the fill is
  gone.
- **`/bar` removal may break the debug snapshot assertion engine** if any
  scenario asserts a `bar_style` field; sweep the scenario files.

## Not built (and why)

- **No OMP `ComposerStyle` registry (box/band/claude/pi/...) port.** The
  operator asked for the input-area *theming*, not the eight composer
  shapes. That is a separate feature; this change makes the existing band
  themeable.
- **No `[tui] bar_style`-replacement key.** `[tui.colors] composer_bg`
  replaces it through the existing role system.
- **No change to `draw_status`'s twelve status branches.** U10's
  classification stands; only the fill goes.