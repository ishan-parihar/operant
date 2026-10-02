# jcode Visual Layer — Full-Parity Plan

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `98ee24f1` (iter-567)
**jcode source**: `parent-projects/jcode` @ `0a9dc7805` — **verified present on disk**
**Parent**: `.omo/plans/reaudit-tui-faithful-jcode-port.md` (gitignored, local only)
**Predecessor**: [`TUI-PORT-NEXT-OUTLINE.md`](TUI-PORT-NEXT-OUTLINE.md) — sequencing only;
this document supersedes its scope estimate.

Every count below was measured at the stated baseline. Where a parent-plan number
disagreed, the measurement won and the discrepancy is recorded.

---

## 0. Headline correction: the surface is 75% larger than budgeted

The parent plan budgets **24,610 LOC** of pixel-deciding jcode code across **5
files**. Measured by counting files that actually construct `Line`/`Span`/`Style`/
`Color` or write into a buffer (excluding `app/tests/`, `ui_tests/`, and all
`*tests.rs`):

| | files | LOC |
|---|---|---|
| Parent plan's budget | 5 | 24,610 |
| **Measured, all production pixel files** | **38** | **44,403** |
| less swarm-only (parent says skip) | 2 | 1,234 |
| **Net parity target** | **36** | **43,169** |

The parent plan **understates by 43%**. Five files it never mentions account for
~13k of the difference: `ui_input.rs` (3,052), `session_picker.rs` (2,451),
`info_widget.rs` (2,131), `ui_inline_image.rs` (1,797), `ui_header.rs` (1,767).

Full parity is therefore a **43k-line** port, not a 25k-line one. Sequencing below
is unchanged; the budget is not.

---

## 1. Second correction: the palette bypass is 84, not ~300

The parent cites "~300 hardcoded colors bypassing a 625-site palette" as measured
cause #6. Measured properly — production code only, excluding `theme_colors.rs`
(palette *definitions* are legitimate literals), excluding `vendor/`, excluding
everything from the first `#[cfg(test)]` onward:

**84 literals across 17 files.** The old figure counted palette definitions and
test fixtures.

| File | count |
|---|---|
| `debug/debug_hub.rs` | 16 |
| `stats_dialog/render.rs` | 9 |
| `bridge_state.rs` | 9 |
| `messages/commands.rs` | 8 |
| `messages/tools.rs` | 7 |
| `messages/markdown_enhanced.rs` | 7 |
| `debug/overlay.rs` | 6 |
| `messages/mod.rs` | 5 |
| `prompt_input/render.rs` | 4 |
| `render/selection.rs` | 3 |
| 8 more files | 1–2 each |

This is a **smaller** job than the plan implies — and it is the cheapest real
parity win available, because 84 literals across 17 files needs no jcode source at
all. `debug/` is not user-facing chrome; do it last or not at all.

**Not a gap**: all 22 jcode palette roles have at least one reader outside
`vendor/` (`Role::User` 33, `System` 7, `Accent` 7, `Ai` 6…). The palette port is
genuinely wired. `Role::Notice` is the one role with **zero** readers — a ported
colour nothing paints, which is the palette-side mirror of the missing renderers.

---

## 2. Tier-by-tier parity ledger

Tier = consequence of omission. **Tier 1 omission looks BROKEN, not plainer.**

### Tier 1 — load-bearing (5)

| # | Item | jcode | operant | Status |
|---|---|---|---|---|
| 1 | Composer glyph mode switch, 4 states | `ui_input.rs:415-426` | 1 constant `PROMPT_POINTER` | **missing** |
| 2 | 10-band chrome stack | `ui.rs:3182-3215` | ad-hoc `reserved: u16 = 1` (`render/mod.rs:136`) | **missing** |
| 3 | User bubble bg `(35,40,50)` + rainbow ordinal | `ui_prepare.rs:369-416` | no bubble, no ordinal | **missing** |
| 4 | Code fence `┌─ lang` / `│ ` / `└─` | `markdown_render_full.rs:475-499` | table borders only | **missing** |
| 5 | Selection = 58% accent blend | `selection_highlight.rs:10-20` | `SELECTION_BLEND = 0.58` | **done** (iter-566) |

### Tier 2 — recognizable but visibly flatter (7 of 13)

| # | Item | Status |
|---|---|---|
| 7 | Command palette zero-height overlay | missing |
| 8 | Cursor by display width | partial — `UnicodeWidthStr` used, no centered recompute |
| 9 | Notice taxonomy, TTLs 3s/8s/5s | missing — colour ported, no taxonomy |
| 10 | Status line compaction + stall/no-token timers | missing |
| 11 | Tool row 3-state icon, errors force detail | missing |
| 12 | 22-role palette | **done** — all roles have readers |
| 13 | Transcript role colour separation | partial — `Role::System` has 7 readers, not applied in `ui_prepare` |

### Tier 3 — polish (7 of 20)

Item 17 (Ctrl+L collapsed transcript) is **confirmed missing**: operant has
`adapter_types/types.rs:71` marked `#[allow(dead_code)] // Prepared for collapsed
read/search display` — the state field exists, the implementation does not.
Items 14–16, 18–20 remain unverified individually; treat as open until checked.

---

## 3. Unbudgeted surfaces the parent plan missed entirely

These decide pixels and have **no operant counterpart at all**:

| jcode file | LOC | operant equivalent |
|---|---|---|
| `ui_header.rs` | 1,767 | **none** — operant has no header renderer |
| `info_widget*.rs` | **8,549** (19 files, excl. tests) | **none** |
| `session_picker.rs` + `/render.rs` | 3,212 | `session_browser.rs` (different design) |
| `ui_inline_image.rs` | 1,797 | `image_paste.rs` only |
| `ui_pinned.rs` | 1,269 | `pinned_images.rs` (partial) |
| `ui_diagram_pane.rs` | 973 | **none** |
| `ui_file_diff.rs` | 615 | `diff_viewer/` (different design) |
| `ui_todo_changes.rs` | 392 | **none** |
| `ui_onboarding.rs` | 745 | **none** |

**These are the parity decisions, not the ports.** Each needs a user-level answer —
"port it", "operant's existing design is better", or "skip" — before anyone writes
a line. Porting all of them is not implied by "visual parity"; *deciding* them is
what full parity requires.

---

## 4. Phased execution

### Phase A — Purge (no jcode source needed) — **do first**

Delete `vendor/render_core` (4,608), `vendor/workspace` (1,246), `vendor/anim`
(940). **Zero consumers** outside `vendor/`; the only mention is a doc comment at
`copy_targets.rs:60`. 6,794 LOC, 92 tests covering code nothing calls.

Then the bridge: `pin_truecolor_for_tests` (7 sites),
`color_depth::detect` ↔ `color::color_capability` (18 sites). jcode has no
opacity/surface model, so this reconciles a model that does not exist upstream.

Keep `vendor/style/` — 44 consumer files.

### Phase B — Palette conformance (no jcode source needed)

Retire the 84 literals in §1 behind `theme_colors` roles. 17 files, mechanical,
and it is the only parity work that can start today without opening the jcode tree.
Start with `messages/` and `stats_dialog/` (user-facing); `debug/` last.

### Phase C — Renderers (the bulk; source available)

Sequential, each slice mutation-proven:

| # | Slice | LOC | Unblocks |
|---|---|---|---|
| C1 | `jcode-tui-render` generic files (`chrome.rs` 91 + `layout.rs` 64 + `lib.rs` 202) | 357 | all overlays |
| C2 | `markdown_render_full.rs` + `markdown_render_support.rs` | 1,585 | **Tier 1 #4** |
| C3 | `ui_tools.rs` | 1,687 | Tier 2 #11 |
| C4 | `ui_prepare.rs` | 2,721 | **Tier 1 #3**, Tier 2 #13 |
| C5 | `ui_messages.rs` | 4,480 | Tier 1 #4 consumer |
| C6 | `ui_input.rs` composer + notice + status | 3,052 | **Tier 1 #1**, Tier 2 #7/#9/#10 |
| C7 | `ui.rs` composition | 3,713 | **Tier 1 #2** |

**C2 before C5 is forced, not preferred**: the `│ ` gutter *is* the copy-target
detection mechanism (`markdown_render_support.rs:42-72`), and `copy_targets.rs`
already depends on that shape. Fence chars changed first silently break copy.

**Do not port**: `swarm_gallery` (3,099), `swarm_tiles` (611), `memory_tiles` (587)
— all three live in `jcode-tui-render/src/`, not `jcode-tui` — plus
`ui_frame_metrics.rs` (1,434 — perf telemetry, only 4 pixel hits),
`markdown_incremental.rs` (311), `markdown_latex_image.rs` (1,294 — operant has
`latex.rs`). **7,336 LOC excluded.**

### Phase D — Composition (finishes iter-549)

10-band chrome replacing `render/mod.rs:136`; kill the second hint row
(`render_status_row` + `render_footer` both draw, jcode has one); route all
overlays through the framework.

### Phase E — Unbudgeted surfaces (§3)

Only after C and D. Each item is a decision first, code second.

### Phase F — Verification

- Rebaseline all 120 goldens **once**, in its own commit, rationale body. Current
  goldens encode operant's layout.
- **Close the `selection-highlight` gap.** It is parked, so the iter-566 blend has
  no golden coverage. Needs a mouse-injection seam on the simulated event pump
  plus a style baseline. Highest-value gate work in the plan — it is the one
  regression that is currently invisible.
- Mutation-prove every ported renderer. Tier 1 #4 in particular: if the fence
  gutter changes, copy silently breaks and no text assertion can see it.

---

## 5. Dependencies

```
A (purge) ──────────────► independent, start now
B (palette) ─────────────► independent, start now
C1 ─► C2 ─► C3 ─► C4 ─► C5 ─► C6 ─► C7
                                   │
                                   ▼
                            D (composition) ─► E (unbudgeted) ─► F (verify)
```

A and B are parallelisable and need no jcode source. C1 gates everything in C.
C2 gates C5.

---

## 6. Decisions needed from the user

1. **§3 unbudgeted surfaces** — port, keep operant's design, or skip? Nine
   surfaces, ~13k LOC. Parity here is a product decision, not an engineering one.
2. **Two known divergences worth a decision**: jcode has vim composer modes and
   operant has `prompt_input/vim.rs` — the parent plan explicitly says *do not
   delete operant's* on jcode's authority. Confirm that stands.
3. **`use_packed` hysteresis** — jcode's packed/scrolling decision is a bare `<=`
   with no dead-band while its scrollbar 40 lines above has explicit
   anti-oscillation hysteresis. The parent says implement packed WITH hysteresis,
   a deliberate divergence from upstream. Confirm.

---

## 7. Standing rules

1. **Mutation-prove every renderer.** iter-566 shipped a test that derived its own
   expectation from the constant under test; `0.58 → 0.30` kept it green. Design
   against that failure mode.
2. **Zero consumers means delete.** `vendor/` reached 6,794 LOC of speculative
   lift because nothing forced the question.
3. **Cite the source line.** The parent plan's citations verified exactly against
   `0a9dc7805`; that is why its conclusions survived two of its numbers being wrong.
4. **Re-measure before quoting.** Both of this document's corrections came from
   measuring a parent-plan claim instead of repeating it.