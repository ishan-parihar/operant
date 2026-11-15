# Porting the settings pane — what "complete and fully functional" requires

Dated-frozen: published `2026-11-16`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**NOTHING IN THIS DOCUMENT IS IMPLEMENTED.** It is an outline. Say go and the first
tranche runs.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All `tui/...` paths are root-relative to that checkout.

Baseline measured at `origin/main` `459c1f82`.

Predecessors, all **complete**:

- `docs/plans/2026-11-14-omp-theming-engine-import.md` — the colour vocabulary.
- `docs/plans/2026-11-15-omp-theming-completion.md` — theme files, 100 palettes, search.
- `docs/plans/plan-2026-11-15-models-and-settings-panels.md` — the first settings panel.

---

## Resume here

The panel exists and works: `/settings` and `/setting` both open it, it has search, a
render-owned scroll window, and one row (bar style) that applies without a restart. The
complaint that opened this pass was **"/setting panel from omp is not ported
effectively"**, and measurement says it is right: the panel is a flat list of **10 rows**
over a configuration surface of **307 leaf fields in 40 settings structs**.

| | Count | Evidence |
|---|---|---|
| Configurable leaf fields | 307 | `crates/operant-core/src/config.rs`, 40 `*Settings` structs |
| Rows the panel offers | 10 | `settings_screen/state.rs` `all_entries()` |
| Fields OMP's panel offers | 10 tabs, ~30 groups | `tui/overlays/settings-defs.ts:22-97` |

**The gap is not cosmetic. The panel is a demo, not a settings surface.** 307 − 10 = 297
settings are reachable only by hand-editing TOML.

---

## What OMP's model actually is

`tui/overlays/settings-defs.ts` defines six setting *types* (`:192-210`) over a base that
carries `tab`, `group`, `label`, `description`, `warning`, `condition` (`:106-127`):

| Type | Shape | Why it matters |
|---|---|---|
| `boolean` | on/off | what operant has |
| `enum` | fixed `values[]` | what operant has |
| `submenu` | `{value,label,description}[]`, optional `onPreview`/`onPreviewCancel` | **the missing one.** A described picker instead of blind cycling |
| `text` | free text, `secret` masks it | credentialed values |
| `multiselect` | toggle list, `ordered` allows reordering | array-valued config |
| `providerLimits` | a nested editor | per-provider rate limits |

Three structural ideas carry the rest:

1. **Tabs** (`:22-32`) — 10 of them, so a 307-field surface is navigable.
2. **Groups** (`:36-97`) — `TAB_GROUPS[tab]` orders heading rows; ungrouped settings
   render first. This is what makes a 40-field tab scannable.
3. **Conditions and warnings** (`:116-127`) — a row can hide itself when irrelevant, and
   a setting that can get the operator rate-limited or banned carries a risk note above
   its description.

The selector (`tui/overlays/settings-selector.ts`, 2016 lines) adds search, a per-tab
detail pane, and the submenu picker with live `onPreview`.

---

## What operant has

`crates/operant-cli/src/tui/settings_screen/` — 2217 lines total.

```
settings_screen.rs          111   SettingsEntry, SettingKind, module wiring
settings_screen/state.rs    639   new/open/apply_and_save/all_entries  ← the 10 rows
settings_screen/keys.rs     182   navigation, edit mode, Enter
settings_screen/render.rs   282   the list, the search line, the scroll window
settings_screen/tests.rs   1003   25 guards
```

`SettingKind` is `Bool | Enum { options } | Number` (`settings_screen.rs:19-23`).
`apply_and_save` (`state.rs:122`) is one `match` over entry keys with a per-field arm;
anything without an arm is silently dropped, which is why `every_settings_entry_has_an_applier`
exists. The render owns its scroll window from the real body height (iter-1917), and
`/theme` now has search + preview to copy from.

**The panel's architecture is sound.** It is the *catalogue* that is missing, plus two
types (`submenu`, `text`) and the tab/group structure.

---

## Work breakdown

Ordered by value per unit of risk. Each unit is one commit, one hand-computed iteration
label, one runnable guard.

### S1 — `submenu`: described choices instead of blind cycling (S)

**Do first.** It is the single change that most makes the panel feel real, and it is
self-contained.

`SettingKind::Enum` currently holds `Vec<&'static str>`. Widen it to carry an optional
description per option, and give `render` a picker overlay: the row shows the current
label, Enter opens a list of `label — description`, Esc cancels without applying.

Rows that need it on day one: `output_style` (4 options), `bar_style` (8 options). Both
already have descriptions available — `SeparatorStyle` has `key()` and a doc comment per
variant, and the output styles are named in `app/commands.rs`.

*Guard:* every `Enum` option set must carry a description, or the picker must say it has
none; and Esc must leave the value untouched. Mutation-prove by making Esc apply.

### S2 — `text` + secret masking (S)

`providerLimits`, API keys, and any path/URL setting. `SettingKind` gains `Text { secret }`.
`prompt_helpers.rs:130` already has the masking idiom (`head…tail`) to reuse.

*Guard:* a secret value is never rendered in full in the row, the detail pane, or the
search index.

### S3 — Tabs and groups (M)

Widen `SettingsEntry` with `tab: SettingsTab` and `group: Option<&'static str>`, add the
tab strip and heading rows, and reuse `theme_screen`'s render-owned window plus its
search. This is where the 307 fields become reachable.

*Guard:* every entry names a tab that exists and a group listed in that tab's group order
(OMP's `TAB_GROUPS` discipline, `settings-defs.ts:36-97`); an unknown tab or group fails
the build, so a new row cannot render under no heading.

### S4 — Fill the catalogue (L, the bulk)

The 40 structs, ordered by what an operator actually reaches for:

| Tab | Struct | Fields | Why |
|---|---|---|---|
| Appearance | `TuiSettings` | 18 | the panel's home turf; colours, bar style, theme |
| Interaction | `BehaviorSettings` | 57 | the largest single struct |
| Tools | `ToolSettings` + `WebToolSettings` | 24 | limits, browser, grep |
| Context | `ContextInjectionSettings` | 13 | what the model sees |
| Skills | `SkillsSettings` | 10 | roots, limits |
| Memory | `MemorySettings` | 8 | |
| Gateway | `GatewaySettings` | 30 | ports, posture |
| Advanced | `AutonomousSettings`, `HolonSettings`, `LoggingSettings`, … | 137 | behind a "show advanced" gate |

Two rules that keep this from becoming 297 hand-written arms:

1. **Generate the arms from the structs, not by hand.** A `macro_rules!` or a build-time
   reflection over `AppConfig` — but see the red-team, item R5.
2. **A field with no apply arm must not be listed.** `every_settings_entry_has_an_applier`
   already enforces this; it is the guard that makes bulk-adding safe.

*Guard:* the existing `every_settings_entry_has_an_applier`, plus a new one that every
serialisable leaf in `AppConfig` is either offered or listed in an explicit
`DELIBERATELY_HIDDEN` set with a reason.

### S5 — Live preview and cancel (M)

OMP's submenu has `onPreview`/`onPreviewCancel`. A settings pane that applies on Enter is
fine; one that previews on cursor movement needs a restore path. `/theme` already does
exactly this (`theme_screen.rs` `restore_target()`), and the read ordering bug that cost a
round there is worth copying as a rule: **read the restore target BEFORE the code that
clears it.**

*Guard:* the `/theme` shape — move, then Esc restores the opener.

### S6 — Risk notes and conditions (S)

`warning` renders above the description in warning styling; `condition` hides a row.
Operant's `show_working_dir`/`show_branch` are already conditional in spirit.

*Guard:* a row with a `warning` renders it; a row whose `condition` is false is absent
from `all_entries` AND from the search result count.

### S7 — The two vocabularies (S, but a decision)

`output_style` in the settings screen offers `default|concise|explanatory|learning`
(`state.rs`), while `/output-style` cycles `auto|stream|verbose`
(`app/commands.rs:1371-1387`). **Two vocabularies for one setting** — a row that writes a
value the command layer does not recognise, or vice versa. This is a correctness bug, not a
polish item, and it must be settled before S1 wires descriptions to it.

*Guard:* one enum, one source of truth, both surfaces read it.

---

## RED-TEAM — how this plan fails

Each of these is drawn from a failure that actually happened in this repository during the
theming and settings passes, not from a general fear of plans. Read this section before
starting S1.

### R1. The catalogue will be generated, and the generator will be wrong in a way nothing catches

`AppConfig` has 307 leaf fields, but many are **not settings an operator should see**:
`config_path`, `api_key`, internal caches, `Option<PathBuf>` plumbing, and fields that are
read once at startup and have no live effect. A generator over the struct will offer them
all.

Worse: operant has **already been burned by exactly this**. `terminal_progress_bar`,
`auto_compact`, `show_turn_duration` and `disable_claude_mds` were offered by
`all_entries`, persisted by the screen, displayed with a value — and **no code outside the
settings screen read them**. The operator could change them, watch them save, and change
nothing (`state.rs` guards, iter-1899). A generated catalogue would resurrect that class at
297× scale.

**Mitigation:** the generator must be *driven by an explicit allow-list*, never by struct
reflection. `DELIBERATELY_HIDDEN` must be non-empty and each entry must carry a reason. A
field enters the panel by being named, not by existing.

**This is the single highest-risk item in the plan.** Budget more for it than for S1–S3
combined.

### R2. A live-preview pane over a process-global palette will flake its own tests

The palette is a process-global (`theme_colors::ACTIVE`, a `RwLock`), and every theming
test mutates it. Two guards in the theming pass failed **only** when run alongside others,
because one test's `set_active_theme` landed between another's read and assert. The fix
was `STYLE_TEST_LOCK` on every test that touches it.

A settings pane with `onPreview` adds a second process-global mutation path (the settings
screen's own state) on top of that one.

**Mitigation:** every new preview test takes `STYLE_TEST_LOCK`; the preview applies to a
*clone* of the palette and commits only on Enter. Write the lock into the first test, not
the fifth.

### R3. `tests` will be written against the wrong string, and the render will be right

In the theming pass, `every_theme_is_reachable_on_a_short_terminal` failed at step 1 for a
**correct render**: the test asserted the theme's *name* (`"light"`) while the row paints
its *label* (`"Light"`). Two rounds were spent discovering the test was the bug.

A settings pane has the same trap at every level: entry key vs label vs value vs the
option's description.

**Mitigation:** assert on the string the row paints (`entry.label`, `option.label`), never
on the underlying key, and say in the test comment which of the four it is.

### R4. A grep-based guard will report live code as dead — twice

Two separate guards were written to prove "every role has a reader". Both were wrong:

1. The first searched only the fully-qualified `vendor::style::theme::<fn>()` form and
   called five **live** roles dead; `ui_theme.rs` and `operant_ui/mod.rs` re-export the
   accessors under short names. The compiler caught it — ten unresolved imports.
2. The second counted the accessor's own `pub fn` and its `pub(super) use` shim as
   readers, so reverting both real call sites still passed.

A "every setting has a reader" guard for the panel will hit the same wall: settings are
read through `Settings`, `AppConfig`, and direct field access, and some are read by the CLI
rather than the TUI.

**Mitigation:** do not write a textual reachability guard for settings. Write a
*behavioural* one — change the setting through the panel, assert the observable changed —
and accept that it covers the rows you can drive, not all 307.

### R5. 297 new arms in one `match` is a maintenance cliff, and the obvious fix is a macro nobody can debug

`apply_and_save` is one `match` with a per-field arm. S4 would take it from 7 arms to
~300. The tempting fix is a macro or a trait-object registry — and then the compiler error
for a missing arm points into macro expansion, and the next agent cannot read it.

**Mitigation:** split by tab into `settings_screen/apply/{appearance,tools,…}.rs`, each a
plain `match` over its own struct's fields. Plain code, no macro. A missing arm is then a
missing arm in a file small enough to hold in one read.

### R6. Style goldens will drift and the drift will be blamed on the wrong unit

The corpus is 68 scenarios / 62 `*.style.txt`. Any palette or layout change drifts them,
and the current baseline already carries **75 text + 8 style drifts from peer churn** that
predate this work. A settings pane that adds a tab strip changes the modal's height, which
changes what is painted behind it.

**Mitigation:** land S3 with the tab strip as its own commit, regenerate deliberately with
`--accept-baseline`, and state in the commit body that the regenerated goldens are from
this change alone. Never accept a baseline you did not just measure.

### R7. The panel will be "complete" and still unreachable

The theming pass's headline finding was that **every capability was shipped and none of it
was reachable from the UI**. The settings panel is the same shape: 307 fields exist, 10 are
reachable. Adding a row to `all_entries` is necessary but not sufficient — the row must be
*applied*, and the thing it configures must be *read*.

**Mitigation:** the acceptance test for this plan is not "the row renders". It is: pick
five settings that are currently TOML-only, drive each from the panel, and assert the
observable behaviour changed. Until that passes, the port is not done.

### R8. Two agents will fix the same thing, and the merge will eat a test

While the theme scroll was being fixed, a peer fixed it independently. The rebase collided
in three files, and resolving it meant **hand-joining two test blocks** in `app/tests.rs` —
one brace off, and the module would not compile.

**Mitigation:** the worktree discipline in AGENTS.md, a hand-computed label per commit, and
one unit per commit. Where two agents touch the same file, prefer taking theirs and adding
only the guard, which is what happened here and worked.

---

## Out of scope

- **A native setup wizard inside the TUI.** `/setup` shells out and now hands over a
  terminal the child can read (iter-2449). Replacing the wizard with an in-TUI flow is a
  separate plan; the wizard does provider auth, model selection and skill seeding, not just
  settings.
- **Writing settings the TUI only reads at startup.** A row whose change needs a restart
  must say so. That is S5's cancel semantics applied honestly, not a new mechanism.
- **The 13 `statusLine*` / OMP tokens with no operant role.** Already recorded as
  deliberately dropped in `theme_bundled.rs`; the settings pane does not change that.

---

## Open questions for the operator

1. **Which 20 settings matter?** S4 is the bulk of the work and 307 fields is too many to
   do blind. If you name the twenty you actually change, the panel is useful in one pass
   instead of three.
2. **Is a tab strip wanted, or a grouped single list?** OMP uses tabs. A single scrolling
   list with headings is less work and, at 307 rows, needs search to be usable — which
   `/theme` already has.
3. **Should `/settings` replace `/setup`, or coexist?** They overlap heavily. Today `/setup`
   is a subprocess wizard and `/settings` is in-TUI; that split is the reason `/setup` was
   fragile in the first place.