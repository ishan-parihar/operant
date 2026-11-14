# Bar styles — separator vocabulary, caps, and whether segments are needed

Dated-frozen: published `2026-11-15`. A plan that changes is a new plan (or declare it
in genome.toml `core.living_plans` and keep a 'Resume here' section).

**STATUS: COMPLETE.** U10–U14 are all resolved — three landed as code, two as
rulings. The table at the foot of this document is the record. Read the U12 and U13
sections before proposing either again: both were closed on measured evidence, not
deferred.

Reference repository (read-only, NOT modified):
`/home/ishanp/Documents/github/my-projects/agentic-harness/external-repos/oh-my-pi`
at `19a76233fb`. All reference paths are root-relative to that checkout.

Predecessor, and the record of what already landed:
`docs/plans/2026-11-14-omp-theming-engine-import.md` (U1–U9 + two audit passes).

---

## Resume here

The predecessor plan is **complete and closed**. It made the *colours* of the UI, the
output text and the syntax themable. This plan covers the half it deliberately left:
**the bar's shape.**

---

## Headline finding

**Colour variety is done. Bar-style variety was built, tested, and then deleted — because
nothing ever asked it for a shape.**

The operator can today choose from 8 palettes and override any of **43** semantic roles
(`[tui.colors]`), and those reach the chrome, the markdown, the syntax colours and the
diff. That half works.

The bar cannot be restyled at all, and the reason is not an oversight in the port — it is
that the mechanism was built twice and wired zero times:

| Artefact | State | Evidence |
|---|---|---|
| `status_line/segments.rs` (292 lines, tested) | **DELETED** | removed in `debe7541` (audit finding 6) |
| `status_line/separators.rs` (178 lines, tested) | **DELETED** | same commit |
| `bar::cap_style()` | **dead** — `#[allow(dead_code)]` | `bar.rs:40`, zero callers |
| `bar::fill_style()` | live, one caller | `ui_input.rs:1266` |
| a `separator` / `preset` config key | **does not exist** | absent from `TuiSettings` |

Both modules are recoverable verbatim from `debe7541^`. The plan doc of the predecessor
says so in code (`status_line/mod.rs:15-21`). So the vocabulary is **not** the work.

### The load-bearing observation

**`draw_status` does not have 51 separators. It has 10, and 6 of them are inside prose.**

The predecessor's audit counted 51 `·` characters in `ui_input.rs` and read that as 51
boundaries to convert. Measuring the function body (`ui_input.rs:907-1240`) gives a
different and much smaller number — and a different shape of work:

| # | Site | Kind | Convertible? |
|---|---|---|---|
| 1 | `:933` ` · +{n} queued` | bar boundary | yes |
| 2 | `:1047` `"{status} · {tps} tps"` | **inside one string** | no — not a boundary |
| 3 | `:1051` `"{status} · ↑{in} ↓{out}"` | **inside one string** | no |
| 4 | `:1067` `"⚠ {miss} cache miss · {status}"` | **inside one string** | no |
| 5 | `:1083` `"network disconnected, waiting to retry · {a} · {b}"` | **one span: prose + 2 qualifiers** | no — not a boundary at all |
| 6 | `:1133` ` · ⚠ {notice}` | bar boundary | yes |
| 7 | `:1146` ` · {label}` | bar boundary | yes |
| 8 | `:1151` ` · {elapsed}` | bar boundary | yes |
| 9 | `:1165` ` · ⚠ {n} cache miss` | bar boundary | yes |
| 10 | `:1234` ` · {hint}` | bar boundary | yes |

**Six of the ten are separator characters inside a single `format!` string.** A powerline
separator cannot be substituted into `"{} · {:.1} tps"` without first splitting that
string into segments — and the tps is not a segment, it is a qualifier on the status
segment. Sites 2–4 are *within-segment punctuation*; site 5 is an error message whose `·`
is prose. Making the bar styleable means deciding which of these are boundaries, and the
honest answer is that **operant's status row is not yet built out of segments** — it is
built out of strings that contain separators.

That is the whole size of the remaining work, and it is why the predecessor's U8 was
deleted rather than half-shipped: the conversion is a change to *what the bar is*, not to
how it is drawn.

---

## Seam sheet — what crosses a boundary today

| # | Field / vocabulary | Producer | Receiver | Live carrier | Population | Duplicate owner |
|---|---|---|---|---|---|---|
| B1 | `ColorPalette` (18 fields) | `theme_colors.rs` | 15 accessors, 48 files | `RwLock<ColorPalette>` | populated, 8 themes | — |
| B2 | `Role` (**43**) | `palette.rs` | `adapt_buffer_for_display` | `RwLock<Option<Palette>>` | 43/43 in `ALL_ROLES`, `key()`, `default_rgb()` | — |
| B3 | `[tui.colors]` overrides | `config.rs` | `apply_role_overrides` | `ROLE_OVERRIDES` cell | **live** | — |
| B4 | `bar::fill_style()` | `bar.rs:29` | `ui_input.rs:1266` | — | live | — |
| B5 | `bar::cap_style()` | `bar.rs:41` | **nobody** | — | **DEAD** | — |
| B6 | separator / preset config key | — | — | — | **DOES NOT EXIST** | — |
| B7 | `status_line::segments` / `separators` | deleted | — | — | recoverable from `debe7541^` | — |
| B8 | `meter()` + `meter_cells()` | `meter.rs` | 4 sites | — | live, one rounding rule | — |

Verified counts, not copied: 43 enum variants = 43 `ALL_ROLES` = 43 `key()` arms =
43 `default_rgb()` arms, no asymmetry in any direction. `from_pairs`
(`palette.rs:257`) accepts any of the 43 keys, so `[tui.colors]` is a genuinely complete
colour surface.

---

## Three ledgers

### Ledger A — what OMP does (verified)

1. **Seven separator styles**, each a named glyph pair plus optional end caps
   (`packages/tui/src/status-line/separators.ts:4-41`):

   | style | left | right | end caps |
   |---|---|---|---|
   | `powerline` | `theme.sep.powerlineLeft` | `powerlineRight` | yes, `useBgAsFg` |
   | `powerline-thin` | `powerlineThinLeft` | `powerlineThinRight` | yes, `useBgAsFg` |
   | `slash` | `/` | `/` | no |
   | `pipe` | `│` | `│` | no |
   | `block` | `█` | `█` | no |
   | `none` | space | space | no |
   | `ascii` | `>` | `<` | no |

   `powerline-thin` is the documented fallback for an unknown style (`:36`).

2. **Glyphs come in two symbol sets** (`packages/tui/src/theme/symbols.ts`): Nerd Font
   powerline `\ue0b0`/`\ue0b2` and thin `\ue0b1`/`\ue0b3` (`:460-463`, `:815-821`), plus
   an ASCII fallback `>`/`<` (`:468-469`). The style names the *shape*; the symbol set
   decides which codepoints draw it.

3. **Caps are the fill colour used as ink** (`status-line/component.ts:3333`):
   `useBgAsFg` rewrites `\x1b[48;` → `\x1b[38;`. In ratatui the two are independent
   fields, so a cap is just the fill as its own foreground — which is exactly what
   `cap_style()` already pins (`bar.rs:41-44`).

4. **Five presets**, each a segment list + separator + per-segment options
   (`status-line/presets.ts`): `default` (powerline-thin, 11 left / 1 right),
   `minimal` (slash, 3 left / 3 right), `compact` (powerline-thin), `full`
   (powerline), `nerd` (powerline, all icons), `ascii`, `custom`. `custom` is omitted
   from the port on purpose: it is the operator's own list, and a default would
   override what they wrote.

5. **Ordered overflow degrade** (`component.ts:3245-3320`): shrink the two `elastic`
   segments toward their floors, then drop from the tail, and **never drop `path`
   first** — OMP documents a naive right-to-left pop that once collapsed the whole bar
   to the model segment, hiding the working directory.

### Ledger B — what operant does (verified)

1. **The bar is one flat `Line`.** `draw_status` (`ui_input.rs:907-1240`) builds spans
   in ~12 `ProcessingStatus` branches and joins them with literal ` · ` inside
   `format!` strings. There is no segment type, no width reflow, no degrade ladder.
2. **The fill exists and is gated on reality.** `Paragraph::style(fill_style())` applies
   only when the caller says the row is real (`mod.rs:3179` →
   `ui_input.rs:1264`), because `draw_status` runs unconditionally and a collapsed
   transcript puts the rect over content. That flag is the single hardest-won line in
   this area — it took two attempts and a mutation test to get right.
3. **The meter is unified.** `meter_cells()` is the one rounding rule; four call sites
   share it.
4. **Width is consulted, but only as a gate.** `occasional_session_history_warning`
   takes `area.width` and returns `None` below 64 columns (`ui_input.rs:720`). Nothing
   reflows the bar to fit.

### Ledger C — the delta, and its consequence

| Capability | OMP | Operant | Consequence |
|---|---|---|---|
| colour vocabulary | 62 fg + 7 bg | **43 roles**, complete config writer | parity is adequate; nothing to do |
| bar fill | `statusLineBg` | live, one call site | done |
| separator style | 7 named styles | **none — 10 hardcoded `·`** | **this plan** |
| end caps | drawn | `cap_style()` exists, **dead** | needs the row to be a bar |
| segment model | data + elastic + degrade | **deleted** | needs the row to be segments |
| symbol set | Nerd + ASCII | **no concept** | decide: one set, or two |

**The delta's shape: operant does not need a bigger vocabulary. It needs the status row
to be made of parts.** Everything else follows from that one change, which is why the
predecessor's U8 was deleted instead of shipped: a segment model with no segments to
model is the definition of an empty seam.

---

## Units, in the order each is shippable alone

### U10 — split the ten sites into boundaries and prose, in code

The smallest unit that changes anything, and the falsifier for everything after it.
Classify each of the 10 `·` sites above as *bar boundary* or *within-segment punctuation*,
write that classification down where the next reader will trip over it, and convert the
**five boundary sites only** (`:933`, `:1133`, `:1146`, `:1151`, `:1165`, `:1234`) to go
through one helper.

- **Files:** `ui_input.rs` (only).
- **Size:** ~40 lines changed, no new module.
- **Mechanism it closes:** the separator has no single definition, so it cannot be
  restyled. One helper is the definition.
- **Guard:** a unit test that the helper's output equals the old literal for every
  style's default, plus `every_separator_site_goes_through_the_helper` — grep-shaped, so
  a sixth hardcoded ` · ` fails the build.
- **Not built:** no config key yet. U10 is behaviour-preserving; if it changes a single
  rendered cell, it is wrong.

### U11 — the separator style key

Restore `separators.rs` from `debe7541^`, give `TuiSettings` a `bar_style: String`
defaulting to `powerline-thin`... **no.** Default to `dot`, because the six unconverted
prose sites mean `powerline-thin` would draw `>` inside `"network disconnected · a · b"`
and read as corruption.

- **Files:** `separators.rs` (restored), `config.rs`, `operant.example.toml`,
  `ui_input.rs` (helper gains a style parameter).
- **Size:** ~120 lines restored + ~15 changed.
- **Guard:** a scenario golden per style is too many; instead one unit test per style
  asserting the rendered boundary string, and one scenario whose golden changes when the
  style changes (mutation-proven, as U5's audit fix was).
- **Not built:** `preset`, `nerd`, symbol sets, caps.

### U12 — decide the symbol set, or skip it

`powerline` needs `\ue0b0`; a terminal without a patched font draws tofu. OMP ships both
sets and picks per theme. Operant has **no symbol-set concept**.

- **Options:** (a) ship Nerd glyphs only and document the requirement; (b) port the two
  sets. (b) is ~200 lines for a font-availability problem that (a) documents in three.
- **Recommendation:** (a). If an operator reports tofu, that is when (b) earns its keep.
- **Guard:** none — this is a decision, not code.

### U13 — the segment model, only if U10/U11 show it is needed

The ordered degrade (`segments.rs`, 292 lines) exists so a bar can drop its tail on a
narrow terminal. Operant's bar is one line with a handful of short labels; at 64 columns
it already fits, and the one width gate that exists (`ui_input.rs:720`) is a *warning*,
not a reflow.

- **Recommendation: do not build this.** It is the largest unit, it is the one the
  predecessor already deleted once, and no measured case needs it. Re-open it only if a
  real terminal width is observed truncating the bar.
- **If built:** restore from `debe7541^`, keep its four `path`-rule guards.

**RULED (iter-2370, after U10/U11): do not build. Verdict confirmed on evidence.**

U10/U11 were the falsifier this unit was gated on, and the measurement says no:

- **The bar's segments are already width-gated where it matters.** `draw_status` consults
  `area.width` at the two places that can overflow — `occasional_session_history_warning`
  returns `None` below 64 columns (`ui_input.rs:720`) and `occasional_status_tip` takes
  `area.width` (`:1249`, `:1260`). The width is already in hand at the paint site.
- **The bar is short.** A status branch is one spinner plus two or three short labels
  (status, elapsed, tokens); the widest is the streaming branch. At 64 columns it fits.
  A degrade ladder that drops the tail protects against a case that does not occur.
- **Nothing reflows because nothing is a list.** The ladder's real job is choosing which
  of *N* segments to drop; `draw_status` is a fixed sequence of at most four spans per
  branch, so "drop the tail" has at most one thing to drop and no ordering to reason
  about.

Re-open only if a real terminal width is observed truncating the status row. If that
happens, the fix is one width check at the paint site — not a 292-line segment model with
a degrade ladder, because the ladder's value is choosing among many segments and the bar
has four.

### U14 — caps

`cap_style()` is dead because a cap needs an *end* to the bar, and the row is a flat
`Paragraph` with no right edge. With U10's helper this becomes ~20 lines. Only worth it
with U11 shipping `powerline` as an option.

- **Files:** `bar.rs`, `ui_input.rs`.
- **Guard:** the existing `a_cap_reuses_the_fill_as_its_ink` already pins the colour
  translation; add the drawn-ness assertion the audit's `terminal-clear` lesson implies.

**LANDED (iter-2372).** The "no right edge" premise above was **wrong**, and finding
that out is what made this a small unit rather than a large one: `bar::fill_style` is
applied to the whole status `Rect`, so the bar's visual right edge already exists at
`area.right() - 1` even though the text inside it is short. The cap sits there and does
not need the text padded out first.

What the 20-line estimate missed is the *spacer*: without padding in the fill colour,
the right cap lands immediately after the text and reads as a stray glyph mid-row rather
than as the bar's edge. `add_caps` fills the gap, and a test asserts the spans account
for every column.

`add_caps` takes the style as a **parameter** rather than reading the process-global
cell, because the cell raced three separate test runs this session. Pure function, no
cell in the tests; the live read is pinned by a separate source-level assertion on the
call site.

---

## Size table

| Unit | File(s) | Size | Signal it worked | Landed |
|---|---|---|---|---|
| U10 boundary split | `separator.rs`, `ui_input.rs` | S | one helper; grep-guard finds no stray ` · ` | **iter-2367** |
| U11 style key | `style.rs` + `config.rs` + example | M | a golden changes when the key changes | **iter-2370** |
| U12 symbol set | ruling only | XS | a written ruling | **ruled (a)** — see U12 |
| U13 segment model | — | — | bar drops its tail under 64 cols | **ruled: do not build** — see U13 |
| U14 caps | `style.rs`, `bar.rs`, `ui_input.rs` | S | a cap glyph at each end of the row | **iter-2372** |

Size is by files, state and guards touched — not effort points. Evidence read for each is
cited inline above.

---

## What is deliberately not built

- **A theme picker.** `/theme` already switches palettes; this plan does not touch it.
- **`custom` preset.** It is the operator's own segment list; a default would override
  what they wrote.
- **`nerd` / `full` / `compact` presets.** Each is a *segment list*, and operant has no
  segment list to arrange. They arrive with U13, if it ever ships.
- **A second symbol set.** U12 recommends against it.
- **62-key colour parity with OMP.** 43 roles cover every surface operant paints; adding
  19 keys nothing reads is the empty-seam failure this plan exists to avoid repeating.
- **Any change to `draw_status`'s 12 branches.** U10 classifies and reroutes; it does not
  restructure. Restructuring is U13's question, and the answer is probably no.

---

## Not checked

- **Live rendering.** No unit here has been run in a terminal. Every claim about what the
  bar looks like is read from source and from the `operant-style-v1` goldens.
- **The full 67-scenario corpus at this commit.** Not re-run for this outline; the
  predecessor's last measurement was 73 drift at `iter-2363`, with
  `skills-view.80x24`/`journey-view.80x24` order-unstable.
- **Font availability on any real terminal.** Whether `\ue0b0` renders is a property of
  the operator's font, not of this repository.
- **`ui_messages.rs` / `context_viz.rs` meters.** They use `meter_cells` but their
  surround was not read for separator style; if the bar gains a style, ask whether those
  surfaces should follow.