# Porting the settings pane — what "complete and fully functional" requires

Dated-frozen: published `2026-11-16`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**NOTHING IN THIS DOCUMENT IS IMPLEMENTED.** It is an outline. Say go and the first
tranche runs.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All `tui/...` paths are root-relative to that checkout.

Baseline measured at `origin/main` `625fa8be`.

---

## Resume here

This plan replaces the first draft, whose headline number was **wrong by a factor of
45**. That draft said the panel exposes **10** settings. It exposes **276**.

The 10 was `grep -c "SettingsEntry {" state.rs` — the count of *hand-declared literals* in
that file. The real panel is those 6 rows PLUS a walk over
`operant_core::config_registry::SETTING_ROWS` filtered by `row.live` and by whether the
generator could type the field (`Widget::None`/`Unsupported` are omitted). Measured by
running `all_entries`:

```
SETTINGS PANEL: 276 rows total, 270 from the registry, 6 declared here
```

The registry itself carries **387 rows, 342 marked `live`**, of which 270 are drawable:
147 `Text`, 85 `Number`, 65 `Bool`, 8 `Cycle`, 79 `None`, 1 `Unsupported`.

**So the catalogue is not the gap. Structure is.** 276 rows in a flat list with no
headings is what makes the panel feel unfinished, and it is the operator's actual
complaint. The first draft's biggest unit — "fill the catalogue with 297 fields" — is
**deleted**: the fields are already there, and generating them would have resurrected
iter-1899's phantom rows at scale.

A guard now pins the real number, so the next reader measures instead of counting:
`settings_screen/tests.rs` prints the drawn count and floors it against `drawable`.

---

## What is actually missing

OMP's selector (`tui/overlays/settings-selector.ts`) and schema
(`tui/overlays/settings-defs.ts`) have six setting types and three structural ideas.
Measured against operant's `SettingKind { Bool, Enum, Number, Text, Unread }`:

| OMP has | Operant has | Verdict |
|---|---|---|
| `boolean` | `Bool` | done |
| `enum` (fixed `values[]`) | `Enum { options }` | done |
| `text` + `secret` masking | `Text` | **kind exists; secret masking does not** |
| `submenu` `{value,label,description}[]` + `onPreview` | none — `Enum` cycles blind | **S1** |
| `multiselect` / `providerLimits` | none | out of scope (no operant array field with a UI) |
| 10 tabs | none — flat list | **S3** |
| `group` heading rows | none | **S3** |
| `warning` risk notes | `Unread` (env-shadowed only) | **S6** |
| `condition` visibility | none | **S6** |

And one correctness bug the first draft found correctly: `output_style` is
`default|concise|explanatory|learning` in the panel but `/output-style` cycles
`auto|stream|verbose`. **S7.**

---

## Work breakdown

Ordered by value per unit of risk. One commit, one hand-computed label, one runnable
guard per unit. S2 (the catalogue) is **deleted**; S4 is renumbered.

### S1 — `submenu`: described choices instead of blind cycling (S)

The single change that most makes the panel feel real.

`SettingKind::Enum` holds `Vec<&'static str>`. Widen the option to carry a description,
and give `render` a picker overlay: the row shows the current label, Enter opens
`label — description`, Esc cancels without applying.

Rows that need it on day one: `output_style` (4 options) and the 8 registry `Cycle` rows,
plus `tui.bar_style` (8 styles, each with a doc comment to draw from).

*Guard:* every `Enum` option set carries a description or the picker says it has none; Esc
leaves the value untouched. Mutation-prove by making Esc apply.

### S2 — Tabs and groups (M) — the highest-value unit

276 rows need structure. Add `tab: SettingsTab` and `group: Option<&'static str>` to
`SettingsEntry`, add the tab strip and heading rows, and reuse the render-owned scroll
window that `settings_screen/render.rs` already has (iter-1917) and that `theme_screen`
now shares.

Tabs, from the registry's own prefixes rather than a hand-kept list: `tui.*` → Appearance,
`agent.*` → Model, `tools.*` → Tools, `mcp.*`/`skills.*` → Skills, `memory.*` → Memory,
`gateway.*` → Gateway, `context.*`/`contextInjection.*` → Context, `autonomous.*` →
Advanced, everything else → General.

*Guard:* every registry key maps to a tab that exists and a group listed in that tab's
group order (OMP's `TAB_GROUPS` discipline, `settings-defs.ts:36-97`). An unknown key
fails the guard, so a new row cannot render under no heading.

### S3 — Risk notes and conditions (S)

`warning` renders above the description in warning styling; `condition` hides a row from
both `all_entries` and the search result count. Operant already has the honest version of
one case: an env-shadowed key becomes `Unread` with a note rather than accepting an edit
that cannot take effect.

*Guard:* a row with a `warning` renders it; a row whose `condition` is false is absent
from the list AND from the match count.

### S4 — Settle the two vocabularies (S, but a correctness bug)

One enum, one source of truth, both surfaces read it. Until this lands, S1 would attach
descriptions to a value set that the command layer does not recognise.

*Guard:* the panel and `/output-style` cannot disagree — one test asserts the command's
cycle list equals the panel's option list.

### Deleted from the first draft

- **"Fill the catalogue" (old S4, 297 fields).** Already done: 270 registry rows draw.
  Re-doing it would double the table and resurrect iter-1899.
- **`multiselect` / `providerLimits`.** No operant field with a single-line editor needs
  them; adding a type with no row is speculative.
- **Live preview for settings (old S5).** `/theme` has it because a theme repaints the
  whole TUI. A settings row applies on Enter, which is the honest semantic; preview would
  add a second process-global mutation path for no operator benefit. See red-team R2.

---

## RED-TEAM — how this plan fails

Each item is drawn from a failure that actually happened in this repository, not from a
general fear of plans.

### R1. The plan itself will be built on an unmeasured number — it already was

The first draft of this document claimed 10 rows, was committed, pushed, and presented.
It was wrong by 45×. The number came from counting source literals instead of running the
code.

**Mitigation:** every quantitative claim in this plan is now a measurement with a command.
The guard in `settings_screen/tests.rs` prints the drawn count so the next reader cannot
repeat it. **If a number in this document and a measurement disagree, the measurement
wins and this document is wrong.**

### R2. A live-preview pane over a process-global palette will flake its own tests

Two theming guards failed **only** when run alongside others, because a neighbouring
test's `set_active_theme` landed between a read and an assert; the fix was `STYLE_TEST_LOCK`
on every test that touches it. A preview adds a second global mutation path on top.

**Mitigation:** this is why old S5 is deleted. If a preview is added anyway, every preview
test takes `STYLE_TEST_LOCK` and the preview applies to a clone, committing only on Enter.

### R3. Tests will be written against the wrong string

In the theming pass `every_theme_is_reachable_on_a_short_terminal` failed at step 1 for a
**correct render**: it asserted the theme's *name* (`"light"`) while the row paints its
*label* (`"Light"`). Two rounds went into discovering the test was the bug.

A settings pane has the same trap at four levels: entry key, label, value, option
description.

**Mitigation:** assert on the string the row paints, and say in the test comment which of
the four it is.

### R4. A grep-based reachability guard will report live code as dead

Two such guards were written for "every role has a reader"; both were wrong. The first
missed short-name re-exports and called five live roles dead. The second counted the
accessor's own definition as a reader, so reverting both real call sites still passed.

**Mitigation:** no textual reachability guard for settings. Drive the row and assert the
observable changed.

### R5. Tabs will be assigned from a prefix table that drifts from the registry

`tab_for(key)` by prefix is deterministic and cheap, but the registry can grow a key whose
prefix nobody mapped, and that row would silently land in General.

**Mitigation:** the guard fails on an unmapped key rather than defaulting. A new registry
row must be given a tab, or the build goes red.

### R6. Style goldens will drift and be blamed on the wrong unit

The corpus is 68 scenarios / 62 `*.style.txt`, and the baseline already carries drift from
peer churn that predates this work. A tab strip changes the modal's height, which changes
what is painted behind it.

**Mitigation:** land the tab strip as its own commit, regenerate with
`--accept-baseline`, and say in the commit body that the regenerated goldens are from this
change alone. Never accept a baseline you did not just measure.

### R7. Two agents will fix the same thing and the merge will eat a test

While the theme scroll was being fixed, a peer fixed it independently. The rebase collided
in three files and the resolution needed **hand-joined test blocks** in `app/tests.rs`.

**Mitigation:** AGENTS.md worktree discipline, one hand-computed label per commit, one unit
per commit. Where two agents touch one file, take theirs and add only the guard.

### R8. The panel will be "complete" and still unreachable

The theming pass's headline finding was that every capability was shipped and none of it
was reachable from the UI. The settings panel has the same shape.

**Mitigation:** the acceptance test is not "the row renders". It is: pick five settings
that were reachable only by hand-editing TOML, drive each from the panel, and assert the
observable behaviour changed.

---

## Out of scope

- **A native setup wizard inside the TUI.** `/setup` shells out and now hands the child a
  terminal it can read (iter-2449). Replacing the wizard is a separate plan.
- **The OMP tokens with no operant role** (per-segment `statusLine*`, `thinking*`,
  `bashMode`). Recorded as deliberately dropped in `theme_bundled.rs`.
- **Reordering (`ordered`) arrays.** No operant field needs it.

---

## Open questions for the operator

1. **Is a tab strip wanted, or a single grouped list?** OMP uses tabs. A single list with
   heading rows is less work and, at 276 rows, needs search to be usable — which the panel
   already has.
2. **Should `/settings` replace `/setup`, or coexist?** They overlap heavily. Today
   `/setup` is a subprocess wizard and `/settings` is in-TUI; that split is why `/setup`
   was fragile in the first place.